use std::process::ExitCode;

// Embeds the Windows icon resource; a no-op for other targets.
fn main() -> ExitCode {
    println!("cargo::rerun-if-changed=packaging/windows/berth.rc");
    println!("cargo::rerun-if-changed=packaging/icons/berth.ico");

    match embed_resource::compile(
        "packaging/windows/berth.rc",
        embed_resource::ParamsIncludeDirs(["packaging/icons"]),
    )
    .manifest_optional()
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("embedding the Windows icon: {error}");
            ExitCode::FAILURE
        }
    }
}
