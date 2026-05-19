fn main() {
    tonic_build::compile_protos("proto/data.proto").unwrap();
}
