pub fn styles() -> String {
    r#"
/* Title base */
.rinch-title {
    font-family: var(--rinch-font-family);
    margin: 0;
    color: var(--rinch-color-text);
}

/* Title levels (h1-h6) */
.rinch-title--1 {
    font-size: var(--rinch-h1-font-size);
    font-weight: var(--rinch-h1-font-weight);
    line-height: var(--rinch-h1-line-height);
}

.rinch-title--2 {
    font-size: var(--rinch-h2-font-size);
    font-weight: var(--rinch-h2-font-weight);
    line-height: var(--rinch-h2-line-height);
}

.rinch-title--3 {
    font-size: var(--rinch-h3-font-size);
    font-weight: var(--rinch-h3-font-weight);
    line-height: var(--rinch-h3-line-height);
}

.rinch-title--4 {
    font-size: var(--rinch-h4-font-size);
    font-weight: var(--rinch-h4-font-weight);
    line-height: var(--rinch-h4-line-height);
}

.rinch-title--5 {
    font-size: var(--rinch-h5-font-size);
    font-weight: var(--rinch-h5-font-weight);
    line-height: var(--rinch-h5-line-height);
}

.rinch-title--6 {
    font-size: var(--rinch-h6-font-size);
    font-weight: var(--rinch-h6-font-weight);
    line-height: var(--rinch-h6-line-height);
}

/* Title alignment */
.rinch-title--left { text-align: left; }
.rinch-title--center { text-align: center; }
.rinch-title--right { text-align: right; }
"#.to_string()
}
