#[cfg(target_os = "linux")]
mod build {
    use libbpf_cargo::SkeletonBuilder;
    use std::{env, path::PathBuf};

    pub(crate) fn build() {
        const SRC: &str = "src/tracer/bpf/lime_tracer.bpf.c";

        let mut out =
            PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set in build script"));

        out.push("lime_tracer.skel.rs");

        SkeletonBuilder::new()
            .source(SRC)
            .build_and_generate(&out)
            .unwrap();
        println!("cargo:rerun-if-changed={}", SRC);
    }
}

#[cfg(not(target_os = "linux"))]
mod build {
    pub(crate) fn build() {}
}

fn main() {
    build::build();

    // Configure and run protobuf compilation
    let mut config = prost_build::Config::new();

    // Optional: Configure the output
    config.bytes(["."]); // Use bytes::Bytes instead of Vec<u8>
    config.type_attribute(".", "#[derive(::serde::Serialize, ::serde::Deserialize)]");

    // Compile the protos
    config
        .compile_protos(&["src/proto/trace_event.proto"], &["src/proto"])
        .unwrap();

    // Tell cargo to rerun if proto files change
    println!("cargo:rerun-if-changed=src/proto/trace_event.proto");
}
