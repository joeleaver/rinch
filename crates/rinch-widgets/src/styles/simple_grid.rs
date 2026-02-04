pub fn styles() -> String {
    r#"
/* SimpleGrid - prevent content from expanding grid columns */
.rinch-simple-grid > * {
    min-width: 0;
}
"#.to_string()
}
