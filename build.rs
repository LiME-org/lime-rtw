#[cfg(target_os = "linux")]
mod build {
    use libbpf_cargo::SkeletonBuilder;
    use std::{
        env,
        path::PathBuf,
        process::{Command, Stdio},
    };

    const SRC: &str = "src/tracer/bpf/lime_tracer.bpf.c";
    const TARGET_VERSION_ENV: &str = "LIME_TARGET_KERNEL_VERSION";

    pub(crate) fn build() {
        let mut out =
            PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set in build script"));

        out.push("lime_tracer.skel.rs");

        let version_code = determine_target_kernel_version_code();
        let clang_args = [
            format!("-DLIME_TARGET_KERNEL_VERSION_CODE={version_code}"),
            format!("-DLINUXKERNEL_VERSION_CODE={version_code}"),
        ];

        let mut builder = SkeletonBuilder::new();
        builder.source(SRC).clang_args(clang_args);

        builder.build_and_generate(&out).unwrap();
        println!("cargo:rerun-if-changed={SRC}");
        println!("cargo:rerun-if-env-changed={TARGET_VERSION_ENV}");
    }

    fn determine_target_kernel_version_code() -> u32 {
        let version_string = env::var(TARGET_VERSION_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(detect_host_kernel_release)
            .unwrap_or_else(|| {
                panic!("Unable to determine kernel version. Set {TARGET_VERSION_ENV} to x.y[.z].")
            });

        parse_kernel_version(&version_string).unwrap_or_else(|| {
            panic!("Failed to parse kernel version '{version_string}'. Expected format x.y[.z].")
        })
    }

    fn detect_host_kernel_release() -> Option<String> {
        let output = Command::new("uname")
            .arg("-r")
            .stdout(Stdio::piped())
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let release = String::from_utf8(output.stdout).ok()?;
        Some(release.trim().to_string())
    }

    fn parse_kernel_version(version: &str) -> Option<u32> {
        let clean_prefix: String = version
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();

        if clean_prefix.is_empty() {
            return None;
        }

        let mut parts = clean_prefix.split('.');
        let major = parts.next()?.parse::<u32>().ok()?;
        let minor = parts.next().unwrap_or("0").parse::<u32>().ok()?;

        let mut patch = 0;
        if let Some(value) = parts.next() {
            patch = value.parse::<u32>().ok()?;
        }

        Some(kernel_version_code(major, minor, patch))
    }

    const fn kernel_version_code(major: u32, minor: u32, patch: u32) -> u32 {
        (major << 16) | (minor << 8) | patch
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
