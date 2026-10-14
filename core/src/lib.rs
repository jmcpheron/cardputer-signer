//! What the signer does that isn't hardware. `no_std` and no heap, so the firmware uses it as is,
//! and the laptop runs its tests (`cargo test`) and draws its screens to PNG
//! (`cargo run --example screens`).
#![cfg_attr(not(test), no_std)]

pub mod atecc;
pub mod blockies;
pub mod buf;
pub mod decode;
pub mod font;
pub mod gfx;
pub mod hex;
pub mod num;
pub mod proto;
pub mod safe_tx;
pub mod screens;
