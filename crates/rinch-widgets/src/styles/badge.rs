pub fn styles() -> String {
    r#"
/* Badge base */
.rinch-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-family: var(--rinch-font-family);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.25px;
    border-radius: 2rem;
    white-space: nowrap;
}

/* Badge sizes */
.rinch-badge--xs {
    height: 1rem;
    padding: 0 0.375rem;
    font-size: 0.5625rem;
}

.rinch-badge--sm {
    height: 1.125rem;
    padding: 0 0.5rem;
    font-size: 0.625rem;
}

.rinch-badge--md {
    height: 1.25rem;
    padding: 0 0.625rem;
    font-size: 0.6875rem;
}

.rinch-badge--lg {
    height: 1.625rem;
    padding: 0 0.75rem;
    font-size: 0.8125rem;
}

.rinch-badge--xl {
    height: 2rem;
    padding: 0 1rem;
    font-size: 0.875rem;
}

/* Badge variants - filled */
.rinch-badge--filled {
    background-color: var(--rinch-primary-color);
    color: white;
}

/* Badge variants - light */
.rinch-badge--light {
    background-color: var(--rinch-primary-color-0);
    color: var(--rinch-primary-color-6);
}

/* Badge variants - outline */
.rinch-badge--outline {
    background-color: transparent;
    color: var(--rinch-primary-color);
    border: 1px solid var(--rinch-primary-color);
}

/* Badge variants - dot */
.rinch-badge--dot {
    background-color: transparent;
    color: var(--rinch-color-text);
    border: 1px solid var(--rinch-color-border);
}

.rinch-badge--dot::before {
    content: '';
    display: inline-block;
    width: 0.5rem;
    height: 0.5rem;
    border-radius: 50%;
    background-color: var(--rinch-primary-color);
    margin-right: 0.375rem;
}

/* Badge with full width */
.rinch-badge--full-width {
    width: 100%;
}

/* Badge color variants - filled */
.rinch-badge--filled[data-color="dark"] { background-color: var(--rinch-color-dark-6); }
.rinch-badge--filled[data-color="gray"] { background-color: var(--rinch-color-gray-6); }
.rinch-badge--filled[data-color="red"] { background-color: var(--rinch-color-red-6); }
.rinch-badge--filled[data-color="pink"] { background-color: var(--rinch-color-pink-6); }
.rinch-badge--filled[data-color="grape"] { background-color: var(--rinch-color-grape-6); }
.rinch-badge--filled[data-color="violet"] { background-color: var(--rinch-color-violet-6); }
.rinch-badge--filled[data-color="indigo"] { background-color: var(--rinch-color-indigo-6); }
.rinch-badge--filled[data-color="blue"] { background-color: var(--rinch-color-blue-6); }
.rinch-badge--filled[data-color="cyan"] { background-color: var(--rinch-color-cyan-6); }
.rinch-badge--filled[data-color="teal"] { background-color: var(--rinch-color-teal-6); }
.rinch-badge--filled[data-color="green"] { background-color: var(--rinch-color-green-6); }
.rinch-badge--filled[data-color="lime"] { background-color: var(--rinch-color-lime-6); color: #000; }
.rinch-badge--filled[data-color="yellow"] { background-color: var(--rinch-color-yellow-6); color: #000; }
.rinch-badge--filled[data-color="orange"] { background-color: var(--rinch-color-orange-6); }

/* Badge color variants - light */
.rinch-badge--light[data-color="dark"] { background-color: var(--rinch-color-dark-0); color: var(--rinch-color-dark-6); }
.rinch-badge--light[data-color="gray"] { background-color: var(--rinch-color-gray-0); color: var(--rinch-color-gray-6); }
.rinch-badge--light[data-color="red"] { background-color: var(--rinch-color-red-0); color: var(--rinch-color-red-6); }
.rinch-badge--light[data-color="pink"] { background-color: var(--rinch-color-pink-0); color: var(--rinch-color-pink-6); }
.rinch-badge--light[data-color="grape"] { background-color: var(--rinch-color-grape-0); color: var(--rinch-color-grape-6); }
.rinch-badge--light[data-color="violet"] { background-color: var(--rinch-color-violet-0); color: var(--rinch-color-violet-6); }
.rinch-badge--light[data-color="indigo"] { background-color: var(--rinch-color-indigo-0); color: var(--rinch-color-indigo-6); }
.rinch-badge--light[data-color="blue"] { background-color: var(--rinch-color-blue-0); color: var(--rinch-color-blue-6); }
.rinch-badge--light[data-color="cyan"] { background-color: var(--rinch-color-cyan-0); color: var(--rinch-color-cyan-6); }
.rinch-badge--light[data-color="teal"] { background-color: var(--rinch-color-teal-0); color: var(--rinch-color-teal-6); }
.rinch-badge--light[data-color="green"] { background-color: var(--rinch-color-green-0); color: var(--rinch-color-green-6); }
.rinch-badge--light[data-color="lime"] { background-color: var(--rinch-color-lime-0); color: var(--rinch-color-lime-6); }
.rinch-badge--light[data-color="yellow"] { background-color: var(--rinch-color-yellow-0); color: var(--rinch-color-yellow-6); }
.rinch-badge--light[data-color="orange"] { background-color: var(--rinch-color-orange-0); color: var(--rinch-color-orange-6); }

/* Badge color variants - outline */
.rinch-badge--outline[data-color="dark"] { color: var(--rinch-color-dark-6); border-color: var(--rinch-color-dark-6); }
.rinch-badge--outline[data-color="gray"] { color: var(--rinch-color-gray-6); border-color: var(--rinch-color-gray-6); }
.rinch-badge--outline[data-color="red"] { color: var(--rinch-color-red-6); border-color: var(--rinch-color-red-6); }
.rinch-badge--outline[data-color="pink"] { color: var(--rinch-color-pink-6); border-color: var(--rinch-color-pink-6); }
.rinch-badge--outline[data-color="grape"] { color: var(--rinch-color-grape-6); border-color: var(--rinch-color-grape-6); }
.rinch-badge--outline[data-color="violet"] { color: var(--rinch-color-violet-6); border-color: var(--rinch-color-violet-6); }
.rinch-badge--outline[data-color="indigo"] { color: var(--rinch-color-indigo-6); border-color: var(--rinch-color-indigo-6); }
.rinch-badge--outline[data-color="blue"] { color: var(--rinch-color-blue-6); border-color: var(--rinch-color-blue-6); }
.rinch-badge--outline[data-color="cyan"] { color: var(--rinch-color-cyan-6); border-color: var(--rinch-color-cyan-6); }
.rinch-badge--outline[data-color="teal"] { color: var(--rinch-color-teal-6); border-color: var(--rinch-color-teal-6); }
.rinch-badge--outline[data-color="green"] { color: var(--rinch-color-green-6); border-color: var(--rinch-color-green-6); }
.rinch-badge--outline[data-color="lime"] { color: var(--rinch-color-lime-6); border-color: var(--rinch-color-lime-6); }
.rinch-badge--outline[data-color="yellow"] { color: var(--rinch-color-yellow-6); border-color: var(--rinch-color-yellow-6); }
.rinch-badge--outline[data-color="orange"] { color: var(--rinch-color-orange-6); border-color: var(--rinch-color-orange-6); }

/* Badge color variants - dot */
.rinch-badge--dot[data-color="dark"]::before { background-color: var(--rinch-color-dark-6); }
.rinch-badge--dot[data-color="gray"]::before { background-color: var(--rinch-color-gray-6); }
.rinch-badge--dot[data-color="red"]::before { background-color: var(--rinch-color-red-6); }
.rinch-badge--dot[data-color="pink"]::before { background-color: var(--rinch-color-pink-6); }
.rinch-badge--dot[data-color="grape"]::before { background-color: var(--rinch-color-grape-6); }
.rinch-badge--dot[data-color="violet"]::before { background-color: var(--rinch-color-violet-6); }
.rinch-badge--dot[data-color="indigo"]::before { background-color: var(--rinch-color-indigo-6); }
.rinch-badge--dot[data-color="blue"]::before { background-color: var(--rinch-color-blue-6); }
.rinch-badge--dot[data-color="cyan"]::before { background-color: var(--rinch-color-cyan-6); }
.rinch-badge--dot[data-color="teal"]::before { background-color: var(--rinch-color-teal-6); }
.rinch-badge--dot[data-color="green"]::before { background-color: var(--rinch-color-green-6); }
.rinch-badge--dot[data-color="lime"]::before { background-color: var(--rinch-color-lime-6); }
.rinch-badge--dot[data-color="yellow"]::before { background-color: var(--rinch-color-yellow-6); }
.rinch-badge--dot[data-color="orange"]::before { background-color: var(--rinch-color-orange-6); }
"#.to_string()
}
