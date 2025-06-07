use prost_build::Config;

fn main() {
    let mut config = Config::new();

    // Derive Serialize, Deserialize, Debug, Eq, PartialEq, Copy, Clone, Hash, Ord, PartialOrd for every enum
    config.type_attribute("copyd.VerifyMode", "#[derive(serde::Serialize, serde::Deserialize)]");
    config.type_attribute("copyd.ExistsAction", "#[derive(serde::Serialize, serde::Deserialize)]");
    config.type_attribute("copyd.CopyEngine", "#[derive(serde::Serialize, serde::Deserialize)]");

    // Enable BTreeMap for maps if needed

    config.compile_protos(&["proto/copyd.proto"], &["proto/"]).unwrap();
}
