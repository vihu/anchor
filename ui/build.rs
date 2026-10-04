//! Compiles the Slint UI in this directory.

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".into());
    slint_build::compile_with_config("app.slint", config).expect("the Slint UI compiles");
}
