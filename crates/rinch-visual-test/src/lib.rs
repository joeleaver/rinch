//! Visual regression testing for rinch.
//!
//! Compares rinch GPU rendering against browser (Chromium) rendering
//! of equivalent HTML/CSS.

pub mod browser;
pub mod capture;
pub mod compare;
pub mod css_export;
pub mod html_serializer;
pub mod report;
pub mod runner;

pub use compare::ComparisonResult;
pub use css_export::computed_style_to_css;
pub use html_serializer::{HtmlConfig, serialize_to_html};
pub use runner::{TestDefinition, TestResult, TestRunner};
