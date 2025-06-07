# copyd Makefile
# Modern file copy daemon with daemon architecture

# Configuration
PREFIX ?= /usr
BINDIR = $(PREFIX)/bin
SYSTEMDDIR = $(PREFIX)/lib/systemd/system
DOCDIR = $(PREFIX)/share/doc/copyd
MANDIR = $(PREFIX)/share/man/man1

# Build configuration
CARGO_FLAGS ?= --release
CARGO_TEST_FLAGS ?= --all-features
TARGET ?= x86_64-unknown-linux-gnu

# Version information
VERSION = $(shell grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
GIT_HASH = $(shell git rev-parse --short HEAD 2>/dev/null || echo "unknown")

# Colors for output
BLUE = \033[34m
GREEN = \033[32m
YELLOW = \033[33m
RED = \033[31m
RESET = \033[0m

.PHONY: all build test install uninstall clean doc package help

# Default target
all: build

help: ## Show this help message
	@echo "$(BLUE)copyd v$(VERSION) - Modern file copy daemon$(RESET)"
	@echo
	@echo "$(GREEN)Available targets:$(RESET)"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  $(BLUE)%-15s$(RESET) %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## Build the project in release mode
	@echo "$(GREEN)Building copyd v$(VERSION) ($(GIT_HASH))...$(RESET)"
	cargo build $(CARGO_FLAGS) --target $(TARGET)
	@echo "$(GREEN)Build complete!$(RESET)"

debug: ## Build the project in debug mode
	@echo "$(GREEN)Building copyd v$(VERSION) in debug mode...$(RESET)"
	cargo build --target $(TARGET)

test: ## Run all tests
	@echo "$(GREEN)Running tests...$(RESET)"
	cargo test $(CARGO_TEST_FLAGS) --target $(TARGET)
	@echo "$(GREEN)Tests completed!$(RESET)"

test-integration: ## Run integration tests only
	@echo "$(GREEN)Running integration tests...$(RESET)"
	cargo test --test integration_tests --target $(TARGET)

test-performance: ## Run performance benchmarks
	@echo "$(GREEN)Running performance benchmarks...$(RESET)"
	cargo test --release test_copy_performance_benchmark --target $(TARGET) -- --nocapture

lint: ## Run clippy linter
	@echo "$(GREEN)Running clippy...$(RESET)"
	cargo clippy --all-targets --all-features --target $(TARGET) -- -D warnings

format: ## Format code with rustfmt
	@echo "$(GREEN)Formatting code...$(RESET)"
	cargo fmt --all

format-check: ## Check code formatting
	@echo "$(GREEN)Checking code formatting...$(RESET)"
	cargo fmt --all -- --check

doc: ## Generate documentation
	@echo "$(GREEN)Generating documentation...$(RESET)"
	cargo doc --no-deps --document-private-items --target $(TARGET)
	@echo "$(GREEN)Documentation generated in target/$(TARGET)/doc/$(RESET)"

audit: ## Run security audit
	@echo "$(GREEN)Running security audit...$(RESET)"
	cargo audit
	@echo "$(GREEN)Checking for unsafe code...$(RESET)"
	@if grep -r "unsafe" --include="*.rs" copyd/src/ copyctl/src/ 2>/dev/null; then \
		echo "$(YELLOW)WARNING: Found unsafe code blocks$(RESET)"; \
	else \
		echo "$(GREEN)No unsafe code blocks found$(RESET)"; \
	fi

install: build ## Install copyd system-wide
	@echo "$(GREEN)Installing copyd v$(VERSION)...$(RESET)"
	install -d $(DESTDIR)$(BINDIR)
	install -d $(DESTDIR)$(SYSTEMDDIR)
	install -d $(DESTDIR)$(DOCDIR)
	
	# Install binaries
	install -m 755 target/$(TARGET)/release/copyd $(DESTDIR)$(BINDIR)/
	install -m 755 target/$(TARGET)/release/copyctl $(DESTDIR)$(BINDIR)/
	
	# Install systemd units
	install -m 644 systemd/copyd.service $(DESTDIR)$(SYSTEMDDIR)/
	install -m 644 systemd/copyd.socket $(DESTDIR)$(SYSTEMDDIR)/
	
	# Install documentation
	install -m 644 README.md $(DESTDIR)$(DOCDIR)/
	install -m 644 DEVELOPMENT_STATUS.md $(DESTDIR)$(DOCDIR)/
	
	@echo "$(GREEN)Installation complete!$(RESET)"
	@echo "$(YELLOW)To enable copyd, run:$(RESET)"
	@echo "  sudo systemctl daemon-reload"
	@echo "  sudo systemctl enable --now copyd.socket"

install-user: build ## Install copyd for current user
	@echo "$(GREEN)Installing copyd v$(VERSION) for user...$(RESET)"
	mkdir -p ~/.local/bin
	mkdir -p ~/.config/systemd/user
	mkdir -p ~/.local/share/doc/copyd
	
	# Install binaries
	cp target/$(TARGET)/release/copyd ~/.local/bin/
	cp target/$(TARGET)/release/copyctl ~/.local/bin/
	
	# Install user systemd units (modified for user session)
	sed 's|/usr/bin/copyd|$(HOME)/.local/bin/copyd|g' systemd/copyd.service > ~/.config/systemd/user/copyd.service
	cp systemd/copyd.socket ~/.config/systemd/user/
	
	# Install documentation
	cp README.md DEVELOPMENT_STATUS.md ~/.local/share/doc/copyd/
	
	@echo "$(GREEN)User installation complete!$(RESET)"
	@echo "$(YELLOW)To enable copyd for your user, run:$(RESET)"
	@echo "  systemctl --user daemon-reload"
	@echo "  systemctl --user enable --now copyd.socket"

uninstall: ## Uninstall copyd system-wide
	@echo "$(YELLOW)Uninstalling copyd...$(RESET)"
	systemctl stop copyd.service copyd.socket 2>/dev/null || true
	systemctl disable copyd.service copyd.socket 2>/dev/null || true
	
	rm -f $(DESTDIR)$(BINDIR)/copyd
	rm -f $(DESTDIR)$(BINDIR)/copyctl
	rm -f $(DESTDIR)$(SYSTEMDDIR)/copyd.service
	rm -f $(DESTDIR)$(SYSTEMDDIR)/copyd.socket
	rm -rf $(DESTDIR)$(DOCDIR)
	
	systemctl daemon-reload 2>/dev/null || true
	@echo "$(GREEN)Uninstation complete!$(RESET)"

clean: ## Clean build artifacts
	@echo "$(GREEN)Cleaning build artifacts...$(RESET)"
	cargo clean
	rm -rf dist/
	rm -f *.deb *.rpm *.tar.gz

dev-setup: ## Set up development environment
	@echo "$(GREEN)Setting up development environment...$(RESET)"
	rustup component add rustfmt clippy
	cargo install cargo-audit
	@echo "$(GREEN)Development environment ready!$(RESET)"

run-daemon: build ## Run copyd daemon in foreground (for development)
	@echo "$(GREEN)Starting copyd daemon...$(RESET)"
	RUST_LOG=debug target/$(TARGET)/release/copyd --foreground

run-demo: build ## Run the demo script
	@echo "$(GREEN)Running copyd demo...$(RESET)"
	cd examples && bash demo.sh

package-deb: build ## Create Debian package
	@echo "$(GREEN)Creating Debian package...$(RESET)"
	mkdir -p dist/deb/usr/bin
	mkdir -p dist/deb/usr/lib/systemd/system
	mkdir -p dist/deb/usr/share/doc/copyd
	mkdir -p dist/deb/DEBIAN
	
	# Copy files
	cp target/$(TARGET)/release/copyd target/$(TARGET)/release/copyctl dist/deb/usr/bin/
	cp systemd/* dist/deb/usr/lib/systemd/system/
	cp README.md DEVELOPMENT_STATUS.md dist/deb/usr/share/doc/copyd/
	
	# Create control file
	echo "Package: copyd" > dist/deb/DEBIAN/control
	echo "Version: $(VERSION)" >> dist/deb/DEBIAN/control
	echo "Section: utils" >> dist/deb/DEBIAN/control
	echo "Priority: optional" >> dist/deb/DEBIAN/control
	echo "Architecture: amd64" >> dist/deb/DEBIAN/control
	echo "Depends: systemd, libc6" >> dist/deb/DEBIAN/control
	echo "Maintainer: copyd project" >> dist/deb/DEBIAN/control
	echo "Description: Modern, high-performance file copy daemon" >> dist/deb/DEBIAN/control
	echo " copyd is a modern replacement for cp/mv with daemon architecture," >> dist/deb/DEBIAN/control
	echo " advanced copy engines, and comprehensive file management features." >> dist/deb/DEBIAN/control
	
	# Create postinst script
	echo "#!/bin/bash" > dist/deb/DEBIAN/postinst
	echo "systemctl daemon-reload" >> dist/deb/DEBIAN/postinst
	echo "systemctl enable copyd.socket" >> dist/deb/DEBIAN/postinst
	echo "echo 'copyd installed. Start with: systemctl start copyd.socket'" >> dist/deb/DEBIAN/postinst
	chmod +x dist/deb/DEBIAN/postinst
	
	# Build package
	fakeroot dpkg-deb --build dist/deb copyd_$(VERSION)_amd64.deb
	@echo "$(GREEN)Debian package created: copyd_$(VERSION)_amd64.deb$(RESET)"

package-tar: build ## Create tarball package
	@echo "$(GREEN)Creating tarball package...$(RESET)"
	mkdir -p dist/copyd-$(VERSION)
	cp target/$(TARGET)/release/copyd target/$(TARGET)/release/copyctl dist/copyd-$(VERSION)/
	cp README.md DEVELOPMENT_STATUS.md dist/copyd-$(VERSION)/
	cp -r systemd examples dist/copyd-$(VERSION)/
	
	cd dist && tar -czf copyd-$(VERSION)-$(TARGET).tar.gz copyd-$(VERSION)/
	@echo "$(GREEN)Tarball created: dist/copyd-$(VERSION)-$(TARGET).tar.gz$(RESET)"

package: package-tar package-deb ## Create all packages

release-prep: clean lint test audit ## Prepare for release
	@echo "$(GREEN)Release preparation complete!$(RESET)"
	@echo "$(YELLOW)Version: $(VERSION)$(RESET)"
	@echo "$(YELLOW)Git hash: $(GIT_HASH)$(RESET)"

# Development targets
watch: ## Watch for changes and rebuild
	@echo "$(GREEN)Watching for changes...$(RESET)"
	cargo watch -x "build --target $(TARGET)"

watch-test: ## Watch for changes and run tests
	@echo "$(GREEN)Watching for changes and running tests...$(RESET)"
	cargo watch -x "test --target $(TARGET)"

coverage: ## Generate test coverage report
	@echo "$(GREEN)Generating coverage report...$(RESET)"
	cargo tarpaulin --out Html --output-dir target/coverage

# System integration
enable: ## Enable and start copyd service
	sudo systemctl daemon-reload
	sudo systemctl enable --now copyd.socket
	@echo "$(GREEN)copyd service enabled and started$(RESET)"

disable: ## Disable and stop copyd service
	sudo systemctl disable --now copyd.socket copyd.service
	@echo "$(YELLOW)copyd service disabled and stopped$(RESET)"

status: ## Show copyd service status
	systemctl status copyd.socket copyd.service

logs: ## Show copyd logs
	journalctl -u copyd.service -f

# Quality assurance
qa: format-check lint test audit ## Run all quality assurance checks
	@echo "$(GREEN)All QA checks passed!$(RESET)"

# Show build information
info: ## Show build information
	@echo "$(BLUE)copyd Build Information$(RESET)"
	@echo "Version: $(VERSION)"
	@echo "Git hash: $(GIT_HASH)"
	@echo "Target: $(TARGET)"
	@echo "Prefix: $(PREFIX)"
	@echo "Cargo flags: $(CARGO_FLAGS)"
	@echo
	@echo "$(BLUE)System Information$(RESET)"
	@uname -a
	@echo
	@echo "$(BLUE)Rust Information$(RESET)"
	@rustc --version
	@cargo --version 