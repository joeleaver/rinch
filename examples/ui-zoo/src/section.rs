//! ui-zoo's sections: the one list the navigation, the content area and
//! anything else that walks them (`examples/bump-calibration`) read.
//!
//! A section's index, the value of the `current_section` signal, is its
//! place in [`Section::ALL`]. The enum and that list are made from one list
//! of names, so they cannot disagree, and every fact about a section is an
//! exhaustive match: a new section does not compile until it has its label,
//! its content and whether it uses media or the network.

use crate::sections::*;
use rinch::prelude::*;

macro_rules! sections {
    ($($name:ident),* $(,)?) => {
        /// A section of ui-zoo, in the order the navigation lists them.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Section {
            $($name),*
        }

        impl Section {
            /// Every section, in order: a section's index is its place here.
            pub const ALL: &'static [Section] = &[$(Section::$name),*];
        }
    };
}

sections!(
    Overview,
    Buttons,
    Inputs,
    Typography,
    Layout,
    Navigation,
    DataDisplay,
    Feedback,
    Overlays,
    Icons,
    Tree,
    RichTextEditor,
    CssFeatures,
    Video,
    RenderSurface,
    ContextMenu,
    FileDrop,
    AudioVideo,
    WebRtc,
    VirtualList,
    Sortable,
);

impl Section {
    /// The section at `index` (`current_section`'s value), if there is one.
    pub fn at(index: usize) -> Option<Section> {
        Self::ALL.get(index).copied()
    }

    /// Its index: its place in [`Section::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// Its name in the navigation.
    pub fn label(self) -> &'static str {
        match self {
            Section::Overview => "Overview",
            Section::Buttons => "Buttons",
            Section::Inputs => "Inputs",
            Section::Typography => "Typography",
            Section::Layout => "Layout",
            Section::Navigation => "Navigation",
            Section::DataDisplay => "Data Display",
            Section::Feedback => "Feedback",
            Section::Overlays => "Overlays",
            Section::Icons => "Icons",
            Section::Tree => "Tree",
            Section::RichTextEditor => "Rich Text Editor",
            Section::CssFeatures => "CSS Features",
            Section::Video => "Video",
            Section::RenderSurface => "RenderSurface",
            Section::ContextMenu => "Context Menu",
            Section::FileDrop => "File Drop",
            Section::AudioVideo => "Audio / Video",
            Section::WebRtc => "WebRTC",
            Section::VirtualList => "Virtual List",
            Section::Sortable => "Sortable",
        }
    }

    /// Whether the section plays or captures media or opens the network
    /// (video, audio and camera, WebRTC). A headless run that should touch no
    /// device but the GPU, such as `examples/bump-calibration`, leaves these
    /// out.
    pub fn uses_media_or_network(self) -> bool {
        match self {
            Section::Video | Section::AudioVideo | Section::WebRtc => true,
            Section::Overview
            | Section::Buttons
            | Section::Inputs
            | Section::Typography
            | Section::Layout
            | Section::Navigation
            | Section::DataDisplay
            | Section::Feedback
            | Section::Overlays
            | Section::Icons
            | Section::Tree
            | Section::RichTextEditor
            | Section::CssFeatures
            | Section::RenderSurface
            | Section::ContextMenu
            | Section::FileDrop
            | Section::VirtualList
            | Section::Sortable => false,
        }
    }

    /// Its content.
    pub fn render(self, __scope: &mut RenderScope) -> NodeHandle {
        match self {
            Section::Overview => overview_section(__scope),
            Section::Buttons => buttons_section(__scope),
            Section::Inputs => inputs_section(__scope),
            Section::Typography => typography_section(__scope),
            Section::Layout => layout_section(__scope),
            Section::Navigation => navigation_section(__scope),
            Section::DataDisplay => data_display_section(__scope),
            Section::Feedback => feedback_section(__scope),
            Section::Overlays => overlays_section(__scope),
            Section::Icons => icons_section(__scope),
            Section::Tree => tree_section(__scope),
            Section::RichTextEditor => editor_section(__scope),
            Section::CssFeatures => css_features_section(__scope),
            Section::Video => video_section(__scope),
            Section::RenderSurface => render_surface_section(__scope),
            Section::ContextMenu => context_menus_section(__scope),
            Section::FileDrop => file_drop_section(__scope),
            Section::AudioVideo => av_section(__scope),
            Section::WebRtc => webrtc_section(__scope),
            Section::VirtualList => virtual_list_section(__scope),
            Section::Sortable => sortable_section(__scope),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Section;

    #[test]
    fn a_sections_index_is_its_place_in_the_list() {
        for (place, section) in Section::ALL.iter().enumerate() {
            assert_eq!(section.index(), place, "{section:?}");
            assert_eq!(Section::at(place), Some(*section));
        }
        assert_eq!(Section::at(Section::ALL.len()), None);
    }
}
