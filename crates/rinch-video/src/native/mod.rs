//! Desktop video backend using libmpv.

pub mod mpv_player;
pub mod frame_upload;
pub mod compositor;

pub use mpv_player::{create_mpv_player, create_mpv_player_paused};
