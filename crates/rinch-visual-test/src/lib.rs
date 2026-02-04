//! Visual regression testing for rinch.
//!
//! Compares rinch GPU rendering against browser (Chromium) rendering
//! of equivalent HTML/CSS.

pub mod css_export;
pub mod html_serializer;
pub mod browser;
pub mod compare;
pub mod capture;
pub mod runner;
pub mod report;

pub use compare::ComparisonResult;
pub use css_export::computed_style_to_css;
pub use html_serializer::{serialize_to_html, HtmlConfig};
pub use runner::{TestRunner, TestDefinition, TestResult};
