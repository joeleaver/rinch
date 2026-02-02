pub fn styles() -> String {
    r#"
/* Center base */
.rinch-center {
    display: flex;
    align-items: center;
    justify-content: center;
}

/* Center inline */
.rinch-center--inline {
    display: inline-flex;
}
"#.to_string()
}
