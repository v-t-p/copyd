fn main() {
    prost_build::compile_protos(&["proto/copyd.proto"], &["proto/"]).unwrap();
}
