use oxc_tailwindcss::{CanonicalizeOptions, DesignSystem, LoadOptions};

fn design() -> DesignSystem {
    DesignSystem::load(
        &LoadOptions::new(env!("CARGO_MANIFEST_DIR")).with_entry_point("tests/fixtures/base.css"),
        1,
    )
    .expect("fixture should load")
}

/// Representative corpus generated with Tailwind CSS 4.3.3's
/// `__unstable__loadDesignSystem().candidatesToCss()` API.
#[test]
fn recognizes_upstream_tailwind_candidates() {
    let design = design();
    let known = [
        "block",
        "inline-block",
        "inline",
        "flex",
        "inline-flex",
        "table",
        "inline-table",
        "table-caption",
        "table-cell",
        "table-column",
        "table-column-group",
        "table-footer-group",
        "table-header-group",
        "table-row-group",
        "table-row",
        "flow-root",
        "grid",
        "inline-grid",
        "contents",
        "list-item",
        "hidden",
        "absolute",
        "fixed",
        "relative",
        "static",
        "sticky",
        "inset-4",
        "-inset-4",
        "inset-x-px",
        "start-full",
        "z-10",
        "-z-10",
        "m-4",
        "-m-4",
        "mx-auto",
        "p-4",
        "p-px",
        "p-1.5",
        "p-[3px]",
        "p-(--space)",
        "space-x-4",
        "divide-x-2",
        "w-4",
        "w-1/2",
        "w-full",
        "w-screen",
        "w-dvw",
        "size-4",
        "min-w-0",
        "max-w-md",
        "h-lvh",
        "basis-1/3",
        "grid-cols-2",
        "grid-cols-[200px_minmax(900px,_1fr)_100px]",
        "col-span-2",
        "col-start-2",
        "row-span-full",
        "auto-cols-fr",
        "gap-4",
        "order-1",
        "-order-1",
        "flex-1",
        "grow",
        "shrink-0",
        "justify-between",
        "items-center",
        "content-stretch",
        "self-auto",
        "place-content-center",
        "font-sans",
        "font-normal",
        "text-sm",
        "text-red-500",
        "text-red-500/50",
        "text-[14px]",
        "leading-none",
        "tracking-wide",
        "line-clamp-2",
        "list-disc",
        "decoration-2",
        "underline-offset-4",
        "bg-red-500",
        "bg-red-500/50",
        "bg-[url('/img.png')]",
        "bg-position-[20%_30%]",
        "from-red-500",
        "via-25%",
        "to-transparent",
        "bg-linear-to-r",
        "border",
        "border-2",
        "border-red-500",
        "border-x-2",
        "rounded-lg",
        "outline",
        "outline-2",
        "ring",
        "ring-2",
        "ring-red-500",
        "shadow-md",
        "opacity-50",
        "blur-sm",
        "brightness-75",
        "drop-shadow-md",
        "backdrop-blur-sm",
        "mix-blend-multiply",
        "bg-blend-overlay",
        "translate-x-4",
        "-translate-y-1/2",
        "rotate-45",
        "-rotate-45",
        "skew-x-6",
        "scale-95",
        "transform-gpu",
        "origin-center",
        "perspective-near",
        "transition",
        "transition-colors",
        "duration-300",
        "ease-in-out",
        "delay-75",
        "animate-spin",
        "will-change-transform",
        "cursor-pointer",
        "pointer-events-none",
        "resize-x",
        "select-none",
        "touch-pan-x",
        "scroll-smooth",
        "snap-x",
        "snap-mandatory",
        "accent-red-500",
        "caret-red-500",
        "hover:flex",
        "focus-visible:ring-2",
        "dark:hover:text-red-500",
        "sm:grid",
        "md:grid",
        "lg:grid",
        "min-sm:grid",
        "min-md:grid",
        "max-sm:hidden",
        "max-md:hidden",
        "max-lg:hidden",
        "supports-[display:grid]:grid",
        "data-[state=open]:block",
        "aria-checked:bg-red-500",
        "group-hover:block",
        "peer-focus:block",
        "[&_p]:block",
        "[@media(width>=10px)]:block",
        "w-1/0",
    ];
    let unknown = [
        "typo",
        "unknown:flex",
        "flex/foo",
        "flex/foo/bar",
        "bg-red-500/50/25",
        "bg-",
        "p-1.1",
        "p-01",
        "-p-4",
        "w-foo",
        "text-no-such-color",
        "sm:typo",
        "[@media(width>=10px){&:hover}]:block",
        "group-sm:block",
        "bg-[]",
        "[Color:red]",
    ];

    let false_negatives =
        known.into_iter().filter(|candidate| !design.is_known_class(candidate)).collect::<Vec<_>>();
    let false_positives = unknown
        .into_iter()
        .filter(|candidate| design.is_known_class(candidate))
        .collect::<Vec<_>>();
    assert!(
        false_negatives.is_empty() && false_positives.is_empty(),
        "false negatives: {false_negatives:?}; false positives: {false_positives:?}"
    );

    let expected_order = [
        "pointer-events-none",
        "absolute",
        "fixed",
        "relative",
        "static",
        "sticky",
        "-inset-4",
        "inset-4",
        "inset-x-px",
        "start-full",
        "-z-10",
        "z-10",
        "-order-1",
        "order-1",
        "col-span-2",
        "col-start-2",
        "row-span-full",
        "-m-4",
        "m-4",
        "mx-auto",
        "line-clamp-2",
        "block",
        "contents",
        "flex",
        "flow-root",
        "grid",
        "hidden",
        "inline",
        "inline-block",
        "inline-flex",
        "inline-grid",
        "inline-table",
        "list-item",
        "table",
        "table-caption",
        "table-cell",
        "table-column",
        "table-column-group",
        "table-footer-group",
        "table-header-group",
        "table-row",
        "table-row-group",
        "size-4",
        "h-lvh",
        "w-1/0",
        "w-1/2",
        "w-4",
        "w-dvw",
        "w-full",
        "w-screen",
        "max-w-md",
        "min-w-0",
        "flex-1",
        "shrink-0",
        "grow",
        "basis-1/3",
        "origin-center",
        "translate-x-4",
        "-translate-y-1/2",
        "scale-95",
        "-rotate-45",
        "rotate-45",
        "skew-x-6",
        "transform-gpu",
        "animate-spin",
        "cursor-pointer",
        "touch-pan-x",
        "resize-x",
        "snap-x",
        "snap-mandatory",
        "list-disc",
        "auto-cols-fr",
        "grid-cols-2",
        "grid-cols-[200px_minmax(900px,_1fr)_100px]",
        "place-content-center",
        "content-stretch",
        "items-center",
        "justify-between",
        "gap-4",
        "space-x-4",
        "divide-x-2",
        "self-auto",
        "scroll-smooth",
        "rounded-lg",
        "border",
        "border-2",
        "border-x-2",
        "border-red-500",
        "bg-red-500",
        "bg-red-500/50",
        "bg-linear-to-r",
        "bg-[url('/img.png')]",
        "from-red-500",
        "via-25%",
        "to-transparent",
        "bg-position-[20%_30%]",
        "p-(--space)",
        "p-1.5",
        "p-4",
        "p-[3px]",
        "p-px",
        "font-sans",
        "text-sm",
        "text-[14px]",
        "leading-none",
        "font-normal",
        "tracking-wide",
        "text-red-500",
        "text-red-500/50",
        "decoration-2",
        "underline-offset-4",
        "caret-red-500",
        "accent-red-500",
        "opacity-50",
        "bg-blend-overlay",
        "mix-blend-multiply",
        "shadow-md",
        "ring",
        "ring-2",
        "ring-red-500",
        "outline",
        "outline-2",
        "blur-sm",
        "brightness-75",
        "drop-shadow-md",
        "backdrop-blur-sm",
        "transition",
        "transition-colors",
        "delay-75",
        "duration-300",
        "ease-in-out",
        "will-change-transform",
        "select-none",
        "perspective-near",
        "group-hover:block",
        "peer-focus:block",
        "hover:flex",
        "focus-visible:ring-2",
        "aria-checked:bg-red-500",
        "data-[state=open]:block",
        "supports-[display:grid]:grid",
        "max-lg:hidden",
        "max-md:hidden",
        "max-sm:hidden",
        "min-sm:grid",
        "sm:grid",
        "md:grid",
        "min-md:grid",
        "lg:grid",
        "dark:hover:text-red-500",
        "[&_p]:block",
        "[@media(width>=10px)]:block",
    ];
    let mut actual_order =
        known.iter().copied().zip(design.class_order(&known)).collect::<Vec<_>>();
    actual_order.sort_unstable_by_key(|(_, order)| *order);
    assert_eq!(
        actual_order.iter().map(|(class_name, _)| *class_name).collect::<Vec<_>>(),
        expected_order
    );
}

#[test]
fn recognizes_upstream_tailwind_edge_candidates() {
    let design = design();
    let known = [
        "bg-left-bottom",
        "object-left-bottom",
        "decoration-clone",
        "-space-x-px",
        "scroll-p-px",
        "block-dvh",
        "inline-svw",
        "max-w-lvh",
        "brightness-0",
        "contrast-125",
        "opacity-0",
        "opacity-12.5",
        "opacity-100",
        "opacity-100.25",
        "rotate-0",
        "scale-0",
        "duration-0",
        "delay-0",
        "grid-cols-1",
        "auto-cols-1.5",
        "z-0",
        "-z-1",
        "order-0",
        "line-clamp-0",
        "col-span-0",
        "col-span-1",
        "row-span-0",
        "grow-0",
        "shrink-0",
        "hue-rotate-0",
        "-hue-rotate-1",
        "rounded-full",
        "origin-top-left",
        "text-sm/6",
        "bg-linear-to-r/oklch",
        "shadow-md/50",
    ];
    let unknown = [
        "brightness-1.5",
        "brightness--1",
        "contrast--1",
        "opacity--1",
        "rotate-1.5",
        "rotate--45",
        "-rotate-1.5",
        "scale-1.5",
        "scale--50",
        "duration-1.5",
        "duration--1",
        "delay-1.5",
        "delay--1",
        "grid-cols-0",
        "grid-cols-1.5",
        "grid-rows-0",
        "z-1.5",
        "z--1",
        "order-1.5",
        "order--1",
        "line-clamp-1.5",
        "grow-1.5",
        "grow--1",
        "shrink-1.5",
        "shrink--1",
        "hue-rotate-1.5",
        "hue-rotate--1",
        "blur-1",
        "blur--1",
        "backdrop-blur-1",
        "saturate-1.5",
        "invert-1.5",
        "rounded-auto",
        "origin-auto",
        "list-square",
        "transition-foo",
        "ease-foo",
        "animate-foo",
        "font-foo",
        "tracking-foo",
        "leading-foo",
        "p-4/foo",
        "w-4/foo",
        "gap-4/foo",
        "opacity-50/foo",
        "rotate-45/foo",
        "font-sans/foo",
        "font-normal/foo",
        "text-sm/foo",
        "text-red-500/foo",
        "bg-red-500/foo",
        "border-2/foo",
        "border-red-500/foo",
        "ring-2/foo",
        "ring-red-500/foo",
        "outline-2/foo",
        "outline-red-500/foo",
        "fill-red-500/foo",
        "from-red-500/foo",
        "via-25%/foo",
        "leading-none/foo",
        "blur-sm/foo",
        "animate-spin/foo",
        "duration-300/foo",
    ];
    let false_negatives = known
        .into_iter()
        .filter(|class_name| !design.is_known_class(class_name))
        .collect::<Vec<_>>();
    let false_positives = unknown
        .into_iter()
        .filter(|class_name| design.is_known_class(class_name))
        .collect::<Vec<_>>();
    assert!(
        false_negatives.is_empty() && false_positives.is_empty(),
        "false negatives: {false_negatives:?}; false positives: {false_positives:?}"
    );
}

#[test]
fn reports_upstream_tailwind_conflicts() {
    let design = design();
    assert_conflicts(
        &design,
        &["p-2", "p-4", "px-2"],
        &[("p-2", "p-4", &["padding"]), ("p-4", "p-2", &["padding"])],
    );
    assert_conflicts(
        &design,
        &["w-4", "w-full", "max-w-md"],
        &[("w-4", "w-full", &["width"]), ("w-full", "w-4", &["width"])],
    );
    assert_conflicts(&design, &["text-sm", "text-red-500", "bg-red-500"], &[]);
    assert_conflicts(
        &design,
        &["ring", "ring-2", "ring-red-500"],
        &[
            ("ring", "ring-2", &["--tw-ring-shadow", "box-shadow"]),
            ("ring-2", "ring", &["--tw-ring-shadow", "box-shadow"]),
        ],
    );
    assert_conflicts(
        &design,
        &["shadow-sm", "shadow-md"],
        &[
            ("shadow-md", "shadow-sm", &["--tw-shadow", "box-shadow"]),
            ("shadow-sm", "shadow-md", &["--tw-shadow", "box-shadow"]),
        ],
    );
    assert_conflicts(
        &design,
        &["translate-x-4", "translate-x-8", "translate-y-4"],
        &[
            ("translate-x-4", "translate-x-8", &["--tw-translate-x", "translate"]),
            ("translate-x-8", "translate-x-4", &["--tw-translate-x", "translate"]),
        ],
    );
    assert_all_conflict(
        &design,
        &["transition", "transition-colors", "transition-opacity"],
        &["transition-property", "transition-timing-function", "transition-duration"],
    );
    assert_conflicts(
        &design,
        &["hover:p-2", "hover:p-4", "focus:p-4"],
        &[("hover:p-2", "hover:p-4", &["padding"]), ("hover:p-4", "hover:p-2", &["padding"])],
    );
    assert_pair_conflict(
        &design,
        "space-x-2",
        "space-x-4",
        &["--tw-sort", "--tw-space-x-reverse", "margin-inline-start", "margin-inline-end"],
    );
    assert_pair_conflict(
        &design,
        "divide-x-2",
        "divide-x-4",
        &[
            "--tw-sort",
            "--tw-divide-x-reverse",
            "border-inline-style",
            "border-inline-start-width",
            "border-inline-end-width",
        ],
    );
    assert_pair_conflict(&design, "bg-red-500", "bg-white", &["background-color"]);
    assert_pair_conflict(
        &design,
        "from-red-500",
        "from-white",
        &["--tw-sort", "--tw-gradient-from", "--tw-gradient-stops"],
    );
    assert_pair_conflict(&design, "scale-x-90", "scale-x-95", &["--tw-scale-x", "scale"]);
    assert_conflicts(&design, &["scale-x-90", "scale-y-90"], &[]);
    assert_pair_conflict(&design, "rotate-x-45", "rotate-x-90", &["--tw-rotate-x", "transform"]);
    assert_conflicts(&design, &["rotate-x-45", "rotate-y-45"], &[]);
    assert_pair_conflict(
        &design,
        "brightness-75",
        "brightness-100",
        &["--tw-brightness", "filter"],
    );
    assert_conflicts(&design, &["brightness-75", "contrast-75"], &[]);
    assert_pair_conflict(&design, "scroll-p-2", "scroll-p-4", &["scroll-padding"]);
    assert_conflicts(&design, &["scroll-p-2", "scroll-px-2"], &[]);
    assert_pair_conflict(&design, "border", "border-2", &["border-style", "border-width"]);
    assert_pair_conflict(
        &design,
        "placeholder-red-500",
        "placeholder-white",
        &["--tw-sort", "color"],
    );
    assert_pair_conflict(&design, "from-25%", "from-50%", &["--tw-gradient-from-position"]);
    assert_conflicts(&design, &["from-25%", "from-red-500"], &[]);
    assert_pair_conflict(
        &design,
        "ring-offset-2",
        "ring-offset-4",
        &["--tw-ring-offset-width", "--tw-ring-offset-shadow"],
    );
}

#[test]
fn canonicalizes_upstream_tailwind_candidates() {
    let design = design();
    let options =
        CanonicalizeOptions { rem: Some(16.0), collapse: true, logical_to_physical: true };
    for (input, expected) in [
        ("[text-wrap:balance]", "text-balance"),
        ("[display:_flex_]", "flex"),
        ("[color:var(--color-red-500)]", "text-red-500"),
        ("[background-color:var(--color-red-500)]", "bg-red-500"),
        ("[color:#fff]", "text-white"),
        ("[color:var(--color-red-500)]/25", "text-red-500/25"),
        ("[color:var(--color-red-500)]/[25%]", "text-red-500/25"),
        ("[color:var(--color-red-500)]/[100%]", "text-red-500"),
        ("[max-height:20%]", "max-h-[20%]"),
        ("[grid-column:2]", "col-2"),
        (
            "[grid-template-columns:repeat(2,minmax(100px,1fr))]",
            "grid-cols-[repeat(2,minmax(100px,1fr))]",
        ),
        ("[grid-template-columns:repeat(2,minmax(0,1fr))]", "grid-cols-2"),
        ("bg-[theme(colors.red.500)]", "bg-red-500"),
        ("bg-[size:theme(spacing.4)]", "bg-size-[--spacing(4)]"),
        ("text-[calc(theme(fontSize.xs)*2)]", "text-[calc(var(--text-xs)*2)]"),
        ("bg-[#FFF]", "bg-white"),
        ("max-[theme(screens.lg)]:flex", "max-[--theme(--breakpoint-lg)]:flex"),
        (
            "grid-cols-[min(50%_,_theme(spacing.80))_auto]",
            "grid-cols-[min(50%,--spacing(80))_auto]",
        ),
        ("pt-[min(20%,calc(var(--spacing)*8))]", "pt-[min(20%,--spacing(8))]"),
        ("pt-[calc(var(--spacing)*8)]", "pt-8"),
        ("max-w-[theme(screens.md)]", "max-w-(--breakpoint-md)"),
        ("w-[theme(maxWidth.md)]", "w-md"),
        ("leading-[1]", "leading-none"),
        ("border-[2px]", "border-2"),
        ("bg-[position:123px]", "bg-position-[123px]"),
        ("bg-[size:123px]", "bg-size-[123px]"),
        ("bg-[123px]", "bg-position-[123px]"),
        ("w-[64rem]", "w-256"),
        ("from-[25%]", "from-25%"),
        ("from-[2.5%]", "from-[2.5%]"),
        ("-mt-[12rem]", "-mt-48"),
        ("-mt-[-12rem]", "mt-48"),
        ("-mt-[12.34rem]", "mt-[-12.34rem]"),
        ("-mt-[-12.34rem]", "mt-[12.34rem]"),
        ("-mt-[492px]", "-mt-123"),
        ("-mt-[-492px]", "mt-123"),
        ("-mt-(--my-var)", "-mt-(--my-var)"),
        ("-mt-[var(--my-var)]", "-mt-(--my-var)"),
        ("mt-[calc(var(--my-var)*-1)]", "-mt-(--my-var)"),
        ("[font-weight:400]", "font-normal"),
        ("[line-height:0]", "leading-0"),
        ("[border-style:solid]", "border-solid"),
        ("focus:[color:#fff]", "focus:text-white"),
        ("[color:#fff]!", "text-white!"),
        ("[@media_print]:block", "print:block"),
        ("[@media_(prefers-color-scheme:_dark)]:block", "dark:block"),
        ("[&:focus]:flex", "focus:flex"),
        ("has-[&:focus]:flex", "has-focus:flex"),
        ("not-[&:focus]:flex", "not-focus:flex"),
        ("group-[&:focus]:flex", "group-focus:flex"),
        ("peer-[&:focus]:flex", "peer-focus:flex"),
        ("data-[selected]:flex", "data-selected:flex"),
        ("aria-[selected=\"true\"]:flex", "aria-selected:flex"),
        ("supports-[gap]:flex", "supports-gap:flex"),
        ("[[data-visible]]:flex", "data-visible:flex"),
        ("[&[data-visible]]:flex", "data-visible:flex"),
        ("[&:first-child]:flex", "first:flex"),
        ("[&:not(:first-child)]:flex", "not-first:flex"),
        ("[&:nth-child(2)]:flex", "nth-2:flex"),
        ("[&:not(:nth-child(2))]:flex", "not-nth-2:flex"),
        ("[&:nth-child(-n+3)]:flex", "nth-[-n+3]:flex"),
        ("[&:nth-last-child(2)]:flex", "nth-last-2:flex"),
        ("[&:nth-child(odd)]:flex", "odd:flex"),
        ("[&:not(:nth-child(odd))]:flex", "even:flex"),
        ("[@media(pointer:fine)]:flex", "pointer-fine:flex"),
        ("[@media_not_(pointer_:_fine)]:flex", "not-pointer-fine:flex"),
        ("[@media_(scripting:_none)]:flex", "noscript:flex"),
        ("start-8", "inset-s-8"),
        ("-end-full", "-inset-e-full"),
        ("bg-gradient-to-r", "bg-linear-to-r"),
        ("order-none", "order-0"),
        ("break-words", "wrap-break-word"),
        ("overflow-ellipsis", "text-ellipsis"),
    ] {
        assert_eq!(
            design.canonicalize_classes(&[input], options),
            [expected],
            "canonicalization for {input}"
        );
    }
}

#[test]
fn collapses_upstream_tailwind_shorthands() {
    let design = design();
    let options =
        CanonicalizeOptions { rem: Some(16.0), collapse: true, logical_to_physical: true };
    for (input, expected) in [
        (&["mt-1", "mr-1", "mb-1", "ml-1"][..], &["m-1"][..]),
        (&["border-t-123", "border-r-123", "border-b-123", "border-l-123"], &["border-123"]),
        (&["border-t-1", "border-r-1", "border-b-1", "border-l-1"], &["border"]),
        (&["mt-1", "mb-1"], &["my-1"]),
        (&["overflow-x-hidden", "overflow-y-hidden"], &["overflow-hidden"]),
        (&["overscroll-x-contain", "overscroll-y-contain"], &["overscroll-contain"]),
        (&["w-4", "h-4"], &["size-4"]),
        (&["w-123", "h-123"], &["size-123"]),
        (&["w-8", "w-8"], &["w-8"]),
        (
            &["w-[calc(1rem+0.25rem)]", "h-[calc(1rem+0.25rem)]", "size-5", "flex"],
            &["size-5", "flex"],
        ),
        (&["hover:w-4", "h-4"], &["hover:w-4", "h-4"]),
        (&["[width:_16px_]", "[height:16px]"], &["size-4"]),
        (&["[font-size:14px]", "[line-height:1.625]"], &["text-sm/relaxed"]),
    ] {
        assert_eq!(design.canonicalize_classes(input, options), expected, "collapse for {input:?}");
    }
}

fn assert_pair_conflict(design: &DesignSystem, left: &str, right: &str, properties: &[&str]) {
    assert_conflicts(
        design,
        &[left, right],
        &[(left, right, properties), (right, left, properties)],
    );
}

fn assert_all_conflict(design: &DesignSystem, classes: &[&str], properties: &[&str]) {
    let mut expected = Vec::new();
    for left in classes {
        for right in classes {
            if left != right {
                expected.push((*left, *right, properties));
            }
        }
    }
    assert_conflicts(design, classes, &expected);
}

fn assert_conflicts(design: &DesignSystem, classes: &[&str], expected: &[(&str, &str, &[&str])]) {
    let mut actual = design
        .conflicting_classes(classes)
        .into_iter()
        .map(|conflict| {
            (
                conflict.class_name.to_string(),
                conflict.conflicting_class_name.to_string(),
                conflict
                    .properties
                    .iter()
                    .map(|property| property.css_property_name.to_string())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = expected
        .iter()
        .map(|(left, right, properties)| {
            (
                (*left).to_owned(),
                (*right).to_owned(),
                properties.iter().map(|property| (*property).to_owned()).collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(actual, expected, "conflicts for {classes:?}");
}
