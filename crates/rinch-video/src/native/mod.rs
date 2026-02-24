//! Desktop video backend using libmpv.

pub mod compositor;
pub mod frame_upload;
pub mod mpv_player;

pub use mpv_player::{create_mpv_player, create_mpv_player_paused};
