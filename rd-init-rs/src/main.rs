#[macro_use]
extern crate nix;

mod init;

fn main() -> anyhow::Result<()> {
    let mut config = init::Config::default();
    config.run()
}
