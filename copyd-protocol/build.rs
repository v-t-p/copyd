use prost_build::Config;

fn main() {
    let mut config = Config::new();

    config.type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");

    // Enable BTreeMap for maps if needed

    config.compile_protos(&["proto/copyd.proto"], &["proto/"]).unwrap();
}
