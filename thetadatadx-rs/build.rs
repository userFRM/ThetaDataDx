#[path = "build_support/mod.rs"]
mod build_support;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    build_support::run()
}
