mod psarc;

use std::fs::File;
use anyhow::{Context, Result};
use crate::psarc::PsArc;

fn main() -> Result<()> {
    let mut args = std::env::args();
    args.next();
    let file = args.next().context("File path not specified")?;
    let file = File::open(file).context("File not found")?;

    let mut psarc = PsArc::new(file)?;
    psarc.unpack()?;
    //println!("{:#?}", psarc.header);
    Ok(())
}
