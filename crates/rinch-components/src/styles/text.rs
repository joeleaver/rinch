pub fn styles() -> String {
    r#"
/* Text base */
.rinch-text {
    font-family: var(--rinch-font-family);
    margin: 0;
}

/* Text sizes */
.rinch-text--xs { font-size: var(--rinch-font-size-xs); line-height: var(--rinch-line-height-xs); }
.rinch-text--sm { font-size: var(--rinch-font-size-sm); line-height: var(--rinch-line-height-sm); }
.rinch-text--md { font-size: var(--rinch-font-size-md); line-height: var(--rinch-line-height-md); }
.rinch-text--lg { font-size: var(--rinch-font-size-lg); line-height: var(--rinch-line-height-lg); }
.rinch-text--xl { font-size: var(--rinch-font-size-xl); line-height: var(--rinch-line-height-xl); }

/* Text weights */
.rinch-text--thin { font-weight: 100; }
.rinch-text--extralight { font-weight: 200; }
.rinch-text--light { font-weight: 300; }
.rinch-text--normal { font-weight: 400; }
.rinch-text--medium { font-weight: 500; }
.rinch-text--semibold { font-weight: 600; }
.rinch-text--bold { font-weight: 700; }
.rinch-text--extrabold { font-weight: 800; }
.rinch-text--black { font-weight: 900; }

/* Text colors */
.rinch-text--primary { color: var(--rinch-primary-color); }
.rinch-text--dimmed { color: var(--rinch-color-dimmed); }
.rinch-text--inherit { color: inherit; }

/* Named colors */
.rinch-text--red { color: var(--rinch-color-red-6); }
.rinch-text--pink { color: var(--rinch-color-pink-6); }
.rinch-text--grape { color: var(--rinch-color-grape-6); }
.rinch-text--violet { color: var(--rinch-color-violet-6); }
.rinch-text--indigo { color: var(--rinch-color-indigo-6); }
.rinch-text--blue { color: var(--rinch-color-blue-6); }
.rinch-text--cyan { color: var(--rinch-color-cyan-6); }
.rinch-text--teal { color: var(--rinch-color-teal-6); }
.rinch-text--green { color: var(--rinch-color-green-6); }
.rinch-text--lime { color: var(--rinch-color-lime-6); }
.rinch-text--yellow { color: var(--rinch-color-yellow-6); }
.rinch-text--orange { color: var(--rinch-color-orange-6); }
.rinch-text--gray { color: var(--rinch-color-gray-6); }
.rinch-text--dark { color: var(--rinch-color-dark-6); }

/* Text alignment */
.rinch-text--left { text-align: left; }
.rinch-text--center { text-align: center; }
.rinch-text--right { text-align: right; }
.rinch-text--justify { text-align: justify; }

/* Text transforms */
.rinch-text--uppercase { text-transform: uppercase; }
.rinch-text--lowercase { text-transform: lowercase; }
.rinch-text--capitalize { text-transform: capitalize; }

/* Text truncate */
.rinch-text--truncate {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

/* Text line clamp */
.rinch-text--line-clamp {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    overflow: hidden;
}
"#
    .to_string()
}
