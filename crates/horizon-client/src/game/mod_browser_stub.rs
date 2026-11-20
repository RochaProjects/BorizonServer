//! Empty browser worker for builds without the legacy client mod registry.
//! The dedicated server still compiles shared screen state, but never runs it.

use super::Game;
use horizon_client::{mod_browser::ModBrowser, mod_list::ModList};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct Jobs;

impl Jobs {
    pub(super) fn browse(&mut self, _ui: &mut ModBrowser, _home: PathBuf) {}

    pub(super) fn install(&mut self, _ui: &mut ModBrowser, _home: PathBuf, _id: String) {}

    pub(super) fn poll(&mut self, _ui: &mut ModBrowser, _mods: &mut ModList) -> bool {
        false
    }
}

impl Game<'_> {
    pub(super) fn drive_mod_browser(&mut self) {}
}
