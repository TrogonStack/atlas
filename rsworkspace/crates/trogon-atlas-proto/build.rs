use std::{env, error::Error, path::PathBuf, process::Command};

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("missing manifest directory")?);
    let proto_dir = manifest_dir.join("../../../proto");
    let descriptor = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing output directory")?)
        .join("descriptor.binpb");

    println!("cargo:rerun-if-changed={}", proto_dir.display());

    let status = Command::new("buf")
        .current_dir(&proto_dir)
        .args(["build", ".", "--as-file-descriptor-set", "--output"])
        .arg(&descriptor)
        .status()
        .map_err(|error| format!("could not run buf; use mise exec -- cargo <command>: {error}"))?;
    if !status.success() {
        return Err(format!("buf descriptor generation failed: {status}").into());
    }
    Ok(())
}
