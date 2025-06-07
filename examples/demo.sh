#!/bin/bash
set -e

echo "=== copyd - Modern Copy Daemon Demo ==="
echo "This script demonstrates the advanced features of copyd"
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

print_section() {
    echo -e "${BLUE}=== $1 ===${NC}"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

print_info() {
    echo -e "${YELLOW}ℹ $1${NC}"
}

print_error() {
    echo -e "${RED}✗ $1${NC}"
}

# Create test directory structure
print_section "Setting up test environment"
TEST_DIR="/tmp/copyd_demo"
SRC_DIR="$TEST_DIR/source"
DST_DIR="$TEST_DIR/destination"

rm -rf "$TEST_DIR"
mkdir -p "$SRC_DIR/subdir1" "$SRC_DIR/subdir2" "$DST_DIR"

# Create various test files
print_info "Creating test files..."

# Regular files of different sizes
echo "Small file content" > "$SRC_DIR/small.txt"
dd if=/dev/urandom of="$SRC_DIR/medium.bin" bs=1M count=10 2>/dev/null
dd if=/dev/urandom of="$SRC_DIR/large.bin" bs=1M count=100 2>/dev/null

# Create a sparse file
print_info "Creating sparse file..."
dd if=/dev/zero of="$SRC_DIR/sparse.img" bs=1M seek=500 count=1 2>/dev/null
print_info "Sparse file created: $(du -sh $SRC_DIR/sparse.img | cut -f1) allocated, $(ls -lh $SRC_DIR/sparse.img | awk '{print $5}') total"

# Create hard links
ln "$SRC_DIR/small.txt" "$SRC_DIR/hardlink.txt"
print_info "Created hard link: $SRC_DIR/hardlink.txt"

# Create symbolic links
ln -s small.txt "$SRC_DIR/symlink.txt"
ln -s ../small.txt "$SRC_DIR/subdir1/parent_symlink.txt"

# Create files in subdirectories
echo "Subdirectory 1 content" > "$SRC_DIR/subdir1/file1.txt"
echo "Subdirectory 2 content" > "$SRC_DIR/subdir2/file2.txt"

# Create files with different permissions
echo "Restricted content" > "$SRC_DIR/restricted.txt"
chmod 600 "$SRC_DIR/restricted.txt"

echo "Executable content" > "$SRC_DIR/executable.sh"
chmod 755 "$SRC_DIR/executable.sh"

print_success "Test environment created"
echo ""

# Function to check if copyd daemon is running
check_daemon() {
    if ! systemctl is-active --quiet copyd; then
        print_error "copyd daemon is not running. Please start it with: sudo systemctl start copyd"
        exit 1
    fi
}

# Function to run copyctl command and show result
run_copyctl() {
    local description="$1"
    shift
    print_info "Running: copyctl $*"
    
    if copyctl "$@"; then
        print_success "$description completed"
    else
        print_error "$description failed"
        return 1
    fi
    echo ""
}

# Check daemon status
print_section "Checking daemon status"
if systemctl is-active --quiet copyd 2>/dev/null; then
    print_success "copyd daemon is running"
    copyctl health
else
    print_info "copyd daemon not running, checking socket activation..."
    if systemctl is-active --quiet copyd.socket 2>/dev/null; then
        print_success "copyd socket is active (will start on demand)"
    else
        print_error "copyd is not running. Please install and start the service."
        echo "Installation commands:"
        echo "  sudo systemctl enable copyd.socket"
        echo "  sudo systemctl start copyd.socket"
        exit 1
    fi
fi
echo ""

# Demonstrate basic copy operations
print_section "Basic Copy Operations"

run_copyctl "Basic file copy" copy "$SRC_DIR/small.txt" "$DST_DIR/"

run_copyctl "Copy with verification" copy "$SRC_DIR/medium.bin" "$DST_DIR/medium_verified.bin" --verify sha256

run_copyctl "Recursive directory copy" copy "$SRC_DIR/subdir1" "$DST_DIR/" --recursive --preserve

# Demonstrate advanced copy engines
print_section "Advanced Copy Engines"

run_copyctl "Copy with reflink (COW)" copy "$SRC_DIR/large.bin" "$DST_DIR/large_reflink.bin" --engine reflink

run_copyctl "Copy with copy_file_range" copy "$SRC_DIR/large.bin" "$DST_DIR/large_cfr.bin" --engine copy_file_range

run_copyctl "Copy with sendfile" copy "$SRC_DIR/medium.bin" "$DST_DIR/medium_sendfile.bin" --engine sendfile

# Demonstrate sparse file handling
print_section "Sparse File Handling"

run_copyctl "Copy sparse file (preserve holes)" copy "$SRC_DIR/sparse.img" "$DST_DIR/sparse_preserved.img" --preserve-sparse

print_info "Comparing sparse file sizes:"
echo "Source sparse file:"
ls -lh "$SRC_DIR/sparse.img" | awk '{print "  Size: " $5 ", Blocks: " $2}'
du -sh "$SRC_DIR/sparse.img" | awk '{print "  Allocated: " $1}'

echo "Destination sparse file:"
ls -lh "$DST_DIR/sparse_preserved.img" | awk '{print "  Size: " $5 ", Blocks: " $2}'
du -sh "$DST_DIR/sparse_preserved.img" | awk '{print "  Allocated: " $1}'
echo ""

# Demonstrate metadata preservation
print_section "Metadata Preservation"

run_copyctl "Copy with full metadata preservation" copy "$SRC_DIR/restricted.txt" "$DST_DIR/" --preserve --preserve-links

print_info "Comparing file permissions:"
ls -l "$SRC_DIR/restricted.txt" | awk '{print "Source:      " $1 " " $3 ":" $4}'
ls -l "$DST_DIR/restricted.txt" | awk '{print "Destination: " $1 " " $3 ":" $4}'
echo ""

# Demonstrate hard link preservation
print_section "Hard Link Preservation"

run_copyctl "Copy directory with hard links" copy "$SRC_DIR" "$DST_DIR/full_copy" --recursive --preserve-links

print_info "Checking hard link preservation:"
src_inode=$(stat -c %i "$SRC_DIR/small.txt")
src_hlink_inode=$(stat -c %i "$SRC_DIR/hardlink.txt")
dst_inode=$(stat -c %i "$DST_DIR/full_copy/source/small.txt")
dst_hlink_inode=$(stat -c %i "$DST_DIR/full_copy/source/hardlink.txt")

echo "Source inodes: small.txt=$src_inode, hardlink.txt=$src_hlink_inode"
echo "Dest inodes:   small.txt=$dst_inode, hardlink.txt=$dst_hlink_inode"

if [ "$src_inode" = "$src_hlink_inode" ] && [ "$dst_inode" = "$dst_hlink_inode" ]; then
    print_success "Hard links preserved correctly"
else
    print_error "Hard link preservation failed"
fi
echo ""

# Demonstrate rate limiting
print_section "Rate Limiting"

print_info "Starting rate-limited copy (10 MB/s)..."
start_time=$(date +%s)
run_copyctl "Rate-limited copy" copy "$SRC_DIR/large.bin" "$DST_DIR/large_limited.bin" --max-rate 10
end_time=$(date +%s)
duration=$((end_time - start_time))

expected_time=10  # 100MB at 10MB/s should take ~10 seconds
print_info "Copy took ${duration}s (expected ~${expected_time}s for rate limiting)"
echo ""

# Demonstrate job management
print_section "Job Management"

print_info "Listing recent jobs:"
copyctl list --completed

print_info "System statistics:"
copyctl stats --days 1

echo ""

# Demonstrate dry run mode
print_section "Dry Run Mode"

run_copyctl "Dry run copy" copy "$SRC_DIR/large.bin" "$DST_DIR/dry_run.bin" --dry-run

if [ ! -f "$DST_DIR/dry_run.bin" ]; then
    print_success "Dry run correctly did not create file"
else
    print_error "Dry run created file when it shouldn't have"
fi
echo ""

# Performance comparison
print_section "Performance Comparison"

print_info "Performance comparison of different engines:"
echo "Testing with $(du -sh $SRC_DIR/large.bin | cut -f1) file..."

# Time different copy methods
time_cp() {
    local method="$1"
    local src="$2"
    local dst="$3"
    shift 3
    
    rm -f "$dst"
    start_time=$(date +%s.%N)
    
    if [ "$method" = "cp" ]; then
        cp "$src" "$dst"
    elif [ "$method" = "rsync" ]; then
        rsync "$src" "$dst"
    else
        copyctl copy "$src" "$dst" "$@" >/dev/null 2>&1
    fi
    
    end_time=$(date +%s.%N)
    duration=$(echo "$end_time - $start_time" | bc)
    size=$(stat -c %s "$src")
    throughput=$(echo "scale=2; $size / 1024 / 1024 / $duration" | bc)
    
    printf "%-15s %8.2fs %8.2f MB/s\n" "$method:" "$duration" "$throughput"
}

echo "Method          Time     Throughput"
echo "--------------------------------"
time_cp "cp" "$SRC_DIR/large.bin" "$DST_DIR/perf_cp.bin"
time_cp "rsync" "$SRC_DIR/large.bin" "$DST_DIR/perf_rsync.bin"
time_cp "copyd (auto)" "$SRC_DIR/large.bin" "$DST_DIR/perf_auto.bin" --engine auto
time_cp "copyd (cfr)" "$SRC_DIR/large.bin" "$DST_DIR/perf_cfr.bin" --engine copy_file_range
time_cp "copyd (reflink)" "$SRC_DIR/large.bin" "$DST_DIR/perf_reflink.bin" --engine reflink

echo ""

# Cleanup
print_section "Cleanup"
print_info "Demo completed! Test files are in $TEST_DIR"
print_info "To clean up: rm -rf $TEST_DIR"

echo ""
print_success "copyd demo completed successfully!"
echo ""
print_info "Next steps:"
echo "  - Try the TUI monitor: copyctl monitor"
echo "  - Try the file navigator: copyctl navigator"
echo "  - View system stats: copyctl stats"
echo "  - Check daemon health: copyctl health" 