use std::path::PathBuf;

fn main() {
    // Build scss file, place in target directory
    std::process::Command::new("dart-sass")
        .arg("styles/lib.scss")
        .arg("target/style.css")
        .status()
        .expect("Failed to build scss file");

    glib_build_tools::compile_resources(
        &["icons"],
        "icons/resources.gresource.xml",
        "icons.gresource",
    );
}
