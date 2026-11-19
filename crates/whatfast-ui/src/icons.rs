//! The icon set of Whatfast, ported from the web client.
//!
//! port: the origin draws its icons with `tauri/src/components/ThemeIcon.tsx`:
//! a `ThemeIconName` and the path of every glyph. The native client keeps the
//! names and the shapes of the origin.
//!
//! The web client paints two sets: the outline of `lucide-solid` in the
//! modern themes and the filled glyph of the original in the skeuomorphic
//! ones. The outline set lives in a package of npm and not in the
//! repository, so this module keeps the filled set, the only one whose paths
//! are in the origin. The native client already paints the outline set with
//! the `IconName` of `gpui_component`.
//!
//! The renderer of GPUI paints an icon as a single monochrome mask, so the
//! three tones of the origin (the shape, the cutout painted with the colour
//! of the panel and the highlight painted with a translucent white) become
//! the shape, a hole and a translucent layer.
//!
//! The path of an icon is the path of its glyph and not the name of a file,
//! so the source of the assets has to build the document of the icon when
//! the renderer asks for it: that is what [`WhatfastAssets`] does.

// The set of icons is a piece of the interface that the application wires in
// its own batch: until then, nothing uses it and the compiler would call
// every icon dead.
#![allow(dead_code)]

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component::IconNamed;

/// The shapes of one glyph of the web client.
struct Glyph {
    /// The path of the glyph: every shape of it, joined.
    path: &'static str,
    /// The opaque shapes.
    main: &'static [&'static str],
    /// The shapes the web client paints with the colour of the panel: in the
    /// native client they are holes.
    cutout: &'static [&'static str],
    /// The translucent shapes, with their opacity.
    overlay: &'static [(&'static str, f64)],
}

const ARCHIVE: Glyph = Glyph {
    path: "M3 7h18v13H3z M2 3h20v5H2z M8 10h8v2H8z M4 4h16v1H4z",
    main: &["M3 7h18v13H3z", "M2 3h20v5H2z"],
    cutout: &["M8 10h8v2H8z"],
    overlay: &[("M4 4h16v1H4z", 0.38)],
};

const ATTACH: Glyph = Glyph {
    path: "M7.2 12.9 14 6.1a3 3 0 0 1 4.2 4.2l-8.5 8.5a5 5 0 0 1-7.1-7.1l8.1-8.1 1.5 1.5-8.1 8.1a2.9 2.9 0 0 0 4.1 4.1l8.5-8.5a.9.9 0 1 0-1.3-1.3l-6.8 6.8a1 1 0 0 0 1.4 1.4l5.1-5.1 1.5 1.5-5.1 5.1a3.1 3.1 0 0 1-4.3-4.3z",
    main: &[
        "M7.2 12.9 14 6.1a3 3 0 0 1 4.2 4.2l-8.5 8.5a5 5 0 0 1-7.1-7.1l8.1-8.1 1.5 1.5-8.1 8.1a2.9 2.9 0 0 0 4.1 4.1l8.5-8.5a.9.9 0 1 0-1.3-1.3l-6.8 6.8a1 1 0 0 0 1.4 1.4l5.1-5.1 1.5 1.5-5.1 5.1a3.1 3.1 0 0 1-4.3-4.3z",
    ],
    cutout: &[],
    overlay: &[],
};

const BELL: Glyph = Glyph {
    path: "M4 17h16l-2-2.7V9a6 6 0 0 0-12 0v5.3z M9 19h6a3 3 0 0 1-6 0z M8 8.5A4.2 4.2 0 0 1 12 5c1.5 0 2.8.7 3.6 1.7A5 5 0 0 0 8 8.5z",
    main: &[
        "M4 17h16l-2-2.7V9a6 6 0 0 0-12 0v5.3z",
        "M9 19h6a3 3 0 0 1-6 0z",
    ],
    cutout: &[],
    overlay: &[(
        "M8 8.5A4.2 4.2 0 0 1 12 5c1.5 0 2.8.7 3.6 1.7A5 5 0 0 0 8 8.5z",
        0.38,
    )],
};

const BOT: Glyph = Glyph {
    path: "M6 8h12a3 3 0 0 1 3 3v6a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3v-6a3 3 0 0 1 3-3z M12 2.4a1.3 1.3 0 1 0 0 2.6a1.3 1.3 0 1 0 0-2.6z M11.4 4.4h1.2V8h-1.2z M8.6 12.6a1.2 1.2 0 1 0 0 2.4a1.2 1.2 0 1 0 0-2.4z M15.4 12.6a1.2 1.2 0 1 0 0 2.4a1.2 1.2 0 1 0 0-2.4z M9.2 16.8h5.6v1.2H9.2z",
    main: &[
        "M6 8h12a3 3 0 0 1 3 3v6a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3v-6a3 3 0 0 1 3-3z",
        "M12 2.4a1.3 1.3 0 1 0 0 2.6a1.3 1.3 0 1 0 0-2.6z",
        "M11.4 4.4h1.2V8h-1.2z",
    ],
    cutout: &[
        "M8.6 12.6a1.2 1.2 0 1 0 0 2.4a1.2 1.2 0 1 0 0-2.4z",
        "M15.4 12.6a1.2 1.2 0 1 0 0 2.4a1.2 1.2 0 1 0 0-2.4z",
        "M9.2 16.8h5.6v1.2H9.2z",
    ],
    overlay: &[],
};

const CHANNELS: Glyph = Glyph {
    path: "M3 5.6C3 3.6 4.9 2 7.3 2h9.4C19.1 2 21 3.6 21 5.6v8.6c0 2-1.9 3.6-4.3 3.6H9.6l-4.4 3.2.7-3.6C4 16.9 3 15.4 3 13.4z M9.5 7.8c-1.5 1.1-2.4 2.5-2.4 4.2s.9 3.1 2.4 4.2l.7-1.2c-1.1-.9-1.7-1.8-1.7-3s.6-2.1 1.7-3z M12 10.5a1.5 1.5 0 1 0 0 3a1.5 1.5 0 1 0 0-3z M14.5 7.8c1.5 1.1 2.4 2.5 2.4 4.2s-.9 3.1-2.4 4.2l-.7-1.2c1.1-.9 1.7-1.8 1.7-3s-.6-2.1-1.7-3z",
    main: &[
        "M3 5.6C3 3.6 4.9 2 7.3 2h9.4C19.1 2 21 3.6 21 5.6v8.6c0 2-1.9 3.6-4.3 3.6H9.6l-4.4 3.2.7-3.6C4 16.9 3 15.4 3 13.4z",
    ],
    cutout: &[
        "M9.5 7.8c-1.5 1.1-2.4 2.5-2.4 4.2s.9 3.1 2.4 4.2l.7-1.2c-1.1-.9-1.7-1.8-1.7-3s.6-2.1 1.7-3z",
        "M12 10.5a1.5 1.5 0 1 0 0 3a1.5 1.5 0 1 0 0-3z",
        "M14.5 7.8c1.5 1.1 2.4 2.5 2.4 4.2s-.9 3.1-2.4 4.2l-.7-1.2c1.1-.9 1.7-1.8 1.7-3s-.6-2.1-1.7-3z",
    ],
    overlay: &[],
};

const CHAT: Glyph = Glyph {
    path: "M3 5.8C3 3.7 5 2 7.4 2h9.2C19 2 21 3.7 21 5.8v7.4c0 2.1-2 3.8-4.4 3.8H11l-5.7 4.2.9-4.5C4.3 16.2 3 14.9 3 13.2z M6.8 5h10.4c.7 0 1.3.4 1.6.9H5.2c.3-.5.9-.9 1.6-.9z",
    main: &[
        "M3 5.8C3 3.7 5 2 7.4 2h9.2C19 2 21 3.7 21 5.8v7.4c0 2.1-2 3.8-4.4 3.8H11l-5.7 4.2.9-4.5C4.3 16.2 3 14.9 3 13.2z",
    ],
    cutout: &[],
    overlay: &[("M6.8 5h10.4c.7 0 1.3.4 1.6.9H5.2c.3-.5.9-.9 1.6-.9z", 0.38)],
};

const CHECK: Glyph = Glyph {
    path: "m2.5 12.5 3.2-3.2 4 4L18.3 4.7l3.2 3.2L9.7 19.7z",
    main: &["m2.5 12.5 3.2-3.2 4 4L18.3 4.7l3.2 3.2L9.7 19.7z"],
    cutout: &[],
    overlay: &[],
};

const CHECK_DOUBLE: Glyph = Glyph {
    path: "m1 12 2.7-2.7 4 4 8.6-8.6L19 7.4 7.7 18.7zm10.4 3.3L20.3 6.4 23 9.1 14.1 18z",
    main: &["m1 12 2.7-2.7 4 4 8.6-8.6L19 7.4 7.7 18.7zm10.4 3.3L20.3 6.4 23 9.1 14.1 18z"],
    cutout: &[],
    overlay: &[],
};

const CLOCK: Glyph = Glyph {
    path: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z M11 5h2v6.2l4.3 2.5-1 1.8-5.3-3.1z M5.5 8A8.2 8.2 0 0 1 12 4c2.6 0 4.9 1.2 6.4 3A9 9 0 0 0 5.5 8z",
    main: &["M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z"],
    cutout: &["M11 5h2v6.2l4.3 2.5-1 1.8-5.3-3.1z"],
    overlay: &[(
        "M5.5 8A8.2 8.2 0 0 1 12 4c2.6 0 4.9 1.2 6.4 3A9 9 0 0 0 5.5 8z",
        0.38,
    )],
};

const CLOSE: Glyph = Glyph {
    path: "m5 3 7 7 7-7 2 2-7 7 7 7-2 2-7-7-7 7-2-2 7-7-7-7z",
    main: &["m5 3 7 7 7-7 2 2-7 7 7 7-2 2-7-7-7 7-2-2 7-7-7-7z"],
    cutout: &[],
    overlay: &[],
};

const COPY: Glyph = Glyph {
    path: "M9 2h11a2 2 0 0 1 2 2v11a2 2 0 0 1 -2 2h-11a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2z M4 7h11a2 2 0 0 1 2 2v11a2 2 0 0 1 -2 2h-11a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2z M9 4h11v1H9zM4 9h11v1H4z",
    main: &["M9 2h11a2 2 0 0 1 2 2v11a2 2 0 0 1 -2 2h-11a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2z"],
    cutout: &[],
    overlay: &[
        (
            "M4 7h11a2 2 0 0 1 2 2v11a2 2 0 0 1 -2 2h-11a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2z",
            0.72,
        ),
        ("M9 4h11v1H9zM4 9h11v1H4z", 0.38),
    ],
};

const DOCUMENT: Glyph = Glyph {
    path: "M5 2h9l5 5v15H5z M14 2v5h5z M8 11h8v1.5H8z M8 15h8v1.5H8z M8 19h5v1.5H8z",
    main: &["M5 2h9l5 5v15H5z"],
    cutout: &[
        "M14 2v5h5z",
        "M8 11h8v1.5H8z",
        "M8 15h8v1.5H8z",
        "M8 19h5v1.5H8z",
    ],
    overlay: &[],
};

const DOWNLOAD: Glyph = Glyph {
    path: "M9 2h6v9h5l-8 8-8-8h5zM3 20h18v3H3z",
    main: &["M9 2h6v9h5l-8 8-8-8h5zM3 20h18v3H3z"],
    cutout: &[],
    overlay: &[],
};

const EDIT: Glyph = Glyph {
    path: "m3 16 13-13a2.8 2.8 0 0 1 4 4L7 20l-5 1z m14.6 5.4 1.8-1.8 4 4-1.8 1.8z",
    main: &["m3 16 13-13a2.8 2.8 0 0 1 4 4L7 20l-5 1z"],
    cutout: &["m14.6 5.4 1.8-1.8 4 4-1.8 1.8z"],
    overlay: &[],
};

const FORWARD: Glyph = Glyph {
    path: "m14 4 8.5 7.5L14 19v-4.3C7.9 14.7 4.7 16.4 1.5 20 2.4 12.8 6.1 8.7 14 8.7z m15.8 7 4.8 4.3-1.1.2-3.7-2.7z",
    main: &["m14 4 8.5 7.5L14 19v-4.3C7.9 14.7 4.7 16.4 1.5 20 2.4 12.8 6.1 8.7 14 8.7z"],
    cutout: &[],
    overlay: &[("m15.8 7 4.8 4.3-1.1.2-3.7-2.7z", 0.38)],
};

const GLOBE: Glyph = Glyph {
    path: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z M5 11h14v2H5zm6-7h2v16h-2z M12 2c3.2 0 5.7 4.4 5.7 10S15.2 22 12 22v-2c1.8 0 3.7-3.2 3.7-8S13.8 4 12 4zm0 0v2c-1.8 0-3.7 3.2-3.7 8s1.9 8 3.7 8v2c-3.2 0-5.7-4.4-5.7-10S8.8 2 12 2z",
    main: &["M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z"],
    cutout: &[
        "M5 11h14v2H5zm6-7h2v16h-2z",
        "M12 2c3.2 0 5.7 4.4 5.7 10S15.2 22 12 22v-2c1.8 0 3.7-3.2 3.7-8S13.8 4 12 4zm0 0v2c-1.8 0-3.7 3.2-3.7 8s1.9 8 3.7 8v2c-3.2 0-5.7-4.4-5.7-10S8.8 2 12 2z",
    ],
    overlay: &[],
};

const IMAGE: Glyph = Glyph {
    path: "M3 4h18a1 1 0 0 1 1 1v14a1 1 0 0 1 -1 1H3a1 1 0 0 1 -1 -1V5a1 1 0 0 1 1 -1z M6.5 8a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z M4 18l5-5 3 3 4-5 4 4z",
    main: &["M3 4h18a1 1 0 0 1 1 1v14a1 1 0 0 1 -1 1H3a1 1 0 0 1 -1 -1V5a1 1 0 0 1 1 -1z"],
    cutout: &[
        "M6.5 8a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z",
        "M4 18l5-5 3 3 4-5 4 4z",
    ],
    overlay: &[],
};

const MAXIMIZE: Glyph = Glyph {
    path: "M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2z M7 8h10v9h-10z",
    main: &["M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2z"],
    cutout: &["M7 8h10v9h-10z"],
    overlay: &[],
};

const MESSAGES: Glyph = Glyph {
    path: "M2.5 5.5A3.5 3.5 0 0 1 6 2h8a3.5 3.5 0 0 1 3.5 3.5v5A3.5 3.5 0 0 1 14 14H9l-4.6 3.2.8-3.6a3.5 3.5 0 0 1-2.7-3.4z M9 16h6.3l3.8 2.7-.7-3a3.2 3.2 0 0 0 3.1-3.2V9A3.5 3.5 0 0 0 18 5.5v5A4.5 4.5 0 0 1 13.5 15H9z",
    main: &[
        "M2.5 5.5A3.5 3.5 0 0 1 6 2h8a3.5 3.5 0 0 1 3.5 3.5v5A3.5 3.5 0 0 1 14 14H9l-4.6 3.2.8-3.6a3.5 3.5 0 0 1-2.7-3.4z",
    ],
    cutout: &[],
    overlay: &[(
        "M9 16h6.3l3.8 2.7-.7-3a3.2 3.2 0 0 0 3.1-3.2V9A3.5 3.5 0 0 0 18 5.5v5A4.5 4.5 0 0 1 13.5 15H9z",
        0.72,
    )],
};

const MIC: Glyph = Glyph {
    path: "M12 2.5a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0v-6a3 3 0 0 1 3-3z M5 11.5a7 7 0 0 0 14 0h-1.4a5.6 5.6 0 0 1-11.2 0z M11.2 18h1.6v3.2h-1.6z M8.7 20.3h6.6v1.3H8.7z",
    main: &[
        "M12 2.5a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0v-6a3 3 0 0 1 3-3z",
        "M5 11.5a7 7 0 0 0 14 0h-1.4a5.6 5.6 0 0 1-11.2 0z",
        "M11.2 18h1.6v3.2h-1.6z",
        "M8.7 20.3h6.6v1.3H8.7z",
    ],
    cutout: &[],
    overlay: &[],
};

const MINIMIZE: Glyph = Glyph {
    path: "M5 11h14a1 1 0 0 1 1 1v1a1 1 0 0 1 -1 1h-14a1 1 0 0 1 -1 -1v-1a1 1 0 0 1 1 -1z",
    main: &["M5 11h14a1 1 0 0 1 1 1v1a1 1 0 0 1 -1 1h-14a1 1 0 0 1 -1 -1v-1a1 1 0 0 1 1 -1z"],
    cutout: &[],
    overlay: &[],
};

const NEW_CHAT: Glyph = Glyph {
    path: "M2 5.5A3.5 3.5 0 0 1 5.5 2h9A3.5 3.5 0 0 1 18 5.5v6a3.5 3.5 0 0 1-3.5 3.5H9l-4.8 3.4.8-3.8a3.5 3.5 0 0 1-3-3.5z M18.5 14v-3h2v3h3v2h-3v3h-2v-3h-3v-2z",
    main: &[
        "M2 5.5A3.5 3.5 0 0 1 5.5 2h9A3.5 3.5 0 0 1 18 5.5v6a3.5 3.5 0 0 1-3.5 3.5H9l-4.8 3.4.8-3.8a3.5 3.5 0 0 1-3-3.5z",
    ],
    cutout: &["M18.5 14v-3h2v3h3v2h-3v3h-2v-3h-3v-2z"],
    overlay: &[],
};

const PALETTE: Glyph = Glyph {
    path: "M12 2C6.2 2 2 5.8 2 10.7 2 15.2 5.5 18 9.2 18h1.1c1 0 1.5.9 1 1.7-.8 1.4.3 2.4 1.9 2.3 5-.5 8.8-4.5 8.8-9.7C22 6.5 17.8 2 12 2z M5.7 9.2a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z M9 6.4a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z M13.5 6.8a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z M16.3 10.5a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z",
    main: &[
        "M12 2C6.2 2 2 5.8 2 10.7 2 15.2 5.5 18 9.2 18h1.1c1 0 1.5.9 1 1.7-.8 1.4.3 2.4 1.9 2.3 5-.5 8.8-4.5 8.8-9.7C22 6.5 17.8 2 12 2z",
    ],
    cutout: &[
        "M5.7 9.2a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z",
        "M9 6.4a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z",
        "M13.5 6.8a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z",
        "M16.3 10.5a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z",
    ],
    overlay: &[],
};

const PIN: Glyph = Glyph {
    path: "m8 2 8 0-1 6 4 4v2h-6v8l-2-2v-6H5v-2l4-4z",
    main: &["m8 2 8 0-1 6 4 4v2h-6v8l-2-2v-6H5v-2l4-4z"],
    cutout: &[],
    overlay: &[],
};

/// The two bars that stop the sound of a voice note.
///
/// The player of the voice note is not in the web client either: the bubble
/// of the reference plays the audio with the `<audio>` tag of the browser,
/// which paints its own controls.
const PAUSE: Glyph = Glyph {
    path: "M8 5h3v14H8z M15 5h3v14h-3z",
    main: &["M8 5h3v14H8z", "M15 5h3v14h-3z"],
    cutout: &[],
    overlay: &[],
};

/// The triangle that starts the sound of a voice note.
const PLAY: Glyph = Glyph {
    path: "M8 5l11 7-11 7z",
    main: &["M8 5l11 7-11 7z"],
    cutout: &[],
    overlay: &[],
};

/// The handset of a call.
///
/// The reference of WhatsApp Desktop draws the call of a conversation in the
/// header, and the web client has no calls: these two glyphs are the only
/// ones of the set that are not in the origin.
const PHONE: Glyph = Glyph {
    path: "M6.62 10.79c1.44 2.83 3.76 5.14 6.59 6.59l2.2-2.2c.27-.27.67-.36 1.02-.24 1.12.37 2.33.57 3.57.57.55 0 1 .45 1 1V20c0 .55-.45 1-1 1C10.61 21 3 13.39 3 4c0-.55.45-1 1-1h3.5c.55 0 1 .45 1 1 0 1.25.2 2.45.57 3.57.11.35.03.74-.25 1.02z",
    main: &[
        "M6.62 10.79c1.44 2.83 3.76 5.14 6.59 6.59l2.2-2.2c.27-.27.67-.36 1.02-.24 1.12.37 2.33.57 3.57.57.55 0 1 .45 1 1V20c0 .55-.45 1-1 1C10.61 21 3 13.39 3 4c0-.55.45-1 1-1h3.5c.55 0 1 .45 1 1 0 1.25.2 2.45.57 3.57.11.35.03.74-.25 1.02z",
    ],
    cutout: &[],
    overlay: &[],
};

const POLL: Glyph = Glyph {
    path: "M5.5 3h13a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-13a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z M7 7h2v8H7zm4 3h2v5h-2zm4-5h2v10h-2zm-8 11h10v2H7z",
    main: &[
        "M5.5 3h13a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-13a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z",
    ],
    cutout: &["M7 7h2v8H7zm4 3h2v5h-2zm4-5h2v10h-2zm-8 11h10v2H7z"],
    overlay: &[],
};

const REPLY: Glyph = Glyph {
    path: "M10 4 1.5 11.5 10 19v-4.3c6.1 0 9.3 1.7 12.5 5.3-.9-7.2-4.6-11.3-12.5-11.3z",
    main: &["M10 4 1.5 11.5 10 19v-4.3c6.1 0 9.3 1.7 12.5 5.3-.9-7.2-4.6-11.3-12.5-11.3z"],
    cutout: &[],
    overlay: &[],
};

const REPLY_PRIVATE: Glyph = Glyph {
    path: "M11.6 7.2a4.2 4.2 0 1 0 8.4 0a4.2 4.2 0 1 0 -8.4 0z M9.3 19.8c.5-4.2 2.7-6.5 6.5-6.5 3.7 0 5.9 2.3 6.4 6.5z M8.7 9.2 4.8 12h4v2H4.9l3.8 2.8L7.4 19 0 13l7.4-6z M13.3 5.8a3.1 3.1 0 0 1 4.8-1.1 3.5 3.5 0 0 0-5.2 2z",
    main: &[
        "M11.6 7.2a4.2 4.2 0 1 0 8.4 0a4.2 4.2 0 1 0 -8.4 0z",
        "M9.3 19.8c.5-4.2 2.7-6.5 6.5-6.5 3.7 0 5.9 2.3 6.4 6.5z",
        "M8.7 9.2 4.8 12h4v2H4.9l3.8 2.8L7.4 19 0 13l7.4-6z",
    ],
    cutout: &[],
    overlay: &[("M13.3 5.8a3.1 3.1 0 0 1 4.8-1.1 3.5 3.5 0 0 0-5.2 2z", 0.38)],
};

const SEARCH: Glyph = Glyph {
    path: "M3.5 10.2a6.7 6.7 0 1 0 13.4 0a6.7 6.7 0 1 0 -13.4 0z m14.8 14.1 1.5-1.5 5.2 5.2a1.1 1.1 0 0 1 0 1.6l-.6.6a1.1 1.1 0 0 1-1.6 0z M6 10.2a4.2 4.2 0 1 0 8.4 0a4.2 4.2 0 1 0 -8.4 0z M6.5 8.8A4.4 4.4 0 0 1 10.4 6c1.2 0 2.3.5 3.1 1.2a5 5 0 0 0-7 1.6z",
    main: &[
        "M3.5 10.2a6.7 6.7 0 1 0 13.4 0a6.7 6.7 0 1 0 -13.4 0z",
        "m14.8 14.1 1.5-1.5 5.2 5.2a1.1 1.1 0 0 1 0 1.6l-.6.6a1.1 1.1 0 0 1-1.6 0z",
    ],
    cutout: &["M6 10.2a4.2 4.2 0 1 0 8.4 0a4.2 4.2 0 1 0 -8.4 0z"],
    overlay: &[(
        "M6.5 8.8A4.4 4.4 0 0 1 10.4 6c1.2 0 2.3.5 3.1 1.2a5 5 0 0 0-7 1.6z",
        0.38,
    )],
};

const SEND: Glyph = Glyph {
    path: "M2 3.2 22 12 2 20.8l2.2-7.3L15 12 4.2 10.5z m4 5.5 14.2 6.2-13.6-3z",
    main: &["M2 3.2 22 12 2 20.8l2.2-7.3L15 12 4.2 10.5z"],
    cutout: &[],
    overlay: &[("m4 5.5 14.2 6.2-13.6-3z", 0.38)],
};

const SETTINGS: Glyph = Glyph {
    path: "m10.2 1.5-.7 2.3-1.6.7-2.2-1.1-2.3 2.3 1.1 2.2-.7 1.6-2.3.7v3.3l2.3.7.7 1.6L3.4 18l2.3 2.3 2.2-1.1 1.6.7.7 2.3h3.3l.7-2.3 1.6-.7 2.2 1.1 2.3-2.3-1.1-2.2.7-1.6 2.3-.7v-3.3l-2.3-.7-.7-1.6 1.1-2.2L18 3.4l-2.2 1.1-1.6-.7-.7-2.3z M8.4 11.8a3.4 3.4 0 1 0 6.8 0a3.4 3.4 0 1 0 -6.8 0z M10.5 3h2l.4 1.5h-2.8z",
    main: &[
        "m10.2 1.5-.7 2.3-1.6.7-2.2-1.1-2.3 2.3 1.1 2.2-.7 1.6-2.3.7v3.3l2.3.7.7 1.6L3.4 18l2.3 2.3 2.2-1.1 1.6.7.7 2.3h3.3l.7-2.3 1.6-.7 2.2 1.1 2.3-2.3-1.1-2.2.7-1.6 2.3-.7v-3.3l-2.3-.7-.7-1.6 1.1-2.2L18 3.4l-2.2 1.1-1.6-.7-.7-2.3z",
    ],
    cutout: &["M8.4 11.8a3.4 3.4 0 1 0 6.8 0a3.4 3.4 0 1 0 -6.8 0z"],
    overlay: &[("M10.5 3h2l.4 1.5h-2.8z", 0.38)],
};

const SHIELD: Glyph = Glyph {
    path: "M12 1.8 21 5v6.2c0 5.2-3.7 9.4-9 11-5.3-1.6-9-5.8-9-11V5z m8.4 11.6 2.1 2.1 4.9-5 1.4 1.5-6.3 6.3L7 13z M12 4 5.4 6.3v4.8c0 .5 0 1 .2 1.5C6.2 8.6 8.6 6 12 4z",
    main: &["M12 1.8 21 5v6.2c0 5.2-3.7 9.4-9 11-5.3-1.6-9-5.8-9-11V5z"],
    cutout: &["m8.4 11.6 2.1 2.1 4.9-5 1.4 1.5-6.3 6.3L7 13z"],
    overlay: &[(
        "M12 4 5.4 6.3v4.8c0 .5 0 1 .2 1.5C6.2 8.6 8.6 6 12 4z",
        0.38,
    )],
};

const SIDEBAR: Glyph = Glyph {
    path: "M4.5 3h15a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-15a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z M8 5h2v14H8z M4 5h3v1H4zm8 0h8v1h-8z",
    main: &[
        "M4.5 3h15a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-15a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z",
    ],
    cutout: &["M8 5h2v14H8z"],
    overlay: &[("M4 5h3v1H4zm8 0h8v1h-8z", 0.38)],
};

const SMILE: Glyph = Glyph {
    path: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z M7 9.2a1.3 1.3 0 1 0 2.6 0a1.3 1.3 0 1 0 -2.6 0z M14.4 9.2a1.3 1.3 0 1 0 2.6 0a1.3 1.3 0 1 0 -2.6 0z M6.6 13.2h10.8c-.6 3.1-2.5 5-5.4 5s-4.8-1.9-5.4-5z M5.5 7.4A8.5 8.5 0 0 1 12 4c2.5 0 4.8 1.1 6.3 2.8A9.1 9.1 0 0 0 5.5 7.4z",
    main: &["M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z"],
    cutout: &[
        "M7 9.2a1.3 1.3 0 1 0 2.6 0a1.3 1.3 0 1 0 -2.6 0z",
        "M14.4 9.2a1.3 1.3 0 1 0 2.6 0a1.3 1.3 0 1 0 -2.6 0z",
        "M6.6 13.2h10.8c-.6 3.1-2.5 5-5.4 5s-4.8-1.9-5.4-5z",
    ],
    overlay: &[(
        "M5.5 7.4A8.5 8.5 0 0 1 12 4c2.5 0 4.8 1.1 6.3 2.8A9.1 9.1 0 0 0 5.5 7.4z",
        0.38,
    )],
};

const SPLIT: Glyph = Glyph {
    path: "M4.5 3h15a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-15a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z M11 4h2v16h-2z M4 5h6v1H4zm10 0h6v1h-6z",
    main: &[
        "M4.5 3h15a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-15a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z",
    ],
    cutout: &["M11 4h2v16h-2z"],
    overlay: &[("M4 5h6v1H4zm10 0h6v1h-6z", 0.38)],
};

const STATUS: Glyph = Glyph {
    path: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z M4.4 12a7.6 7.6 0 1 0 15.2 0a7.6 7.6 0 1 0 -15.2 0z",
    main: &["M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z"],
    cutout: &["M4.4 12a7.6 7.6 0 1 0 15.2 0a7.6 7.6 0 1 0 -15.2 0z"],
    overlay: &[],
};

const STICKER: Glyph = Glyph {
    path: "M3 2h18v12.5L13.5 22H3z M13 14h6.5L13 20.5z M6.8 8a1.2 1.2 0 1 0 2.4 0a1.2 1.2 0 1 0 -2.4 0z M13.3 8a1.2 1.2 0 1 0 2.4 0a1.2 1.2 0 1 0 -2.4 0z M7 11h8c-.6 2-1.9 3-4 3s-3.4-1-4-3z",
    main: &["M3 2h18v12.5L13.5 22H3z"],
    cutout: &[
        "M13 14h6.5L13 20.5z",
        "M6.8 8a1.2 1.2 0 1 0 2.4 0a1.2 1.2 0 1 0 -2.4 0z",
        "M13.3 8a1.2 1.2 0 1 0 2.4 0a1.2 1.2 0 1 0 -2.4 0z",
        "M7 11h8c-.6 2-1.9 3-4 3s-3.4-1-4-3z",
    ],
    overlay: &[],
};

const STORAGE: Glyph = Glyph {
    path: "M4 3h16a2 2 0 0 1 2 2v3a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-3a2 2 0 0 1 2 -2z M4 13h16a2 2 0 0 1 2 2v4a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-4a2 2 0 0 1 2 -2z M16.7 6.5a1.3 1.3 0 1 0 2.6 0a1.3 1.3 0 1 0 -2.6 0z M16.7 17a1.3 1.3 0 1 0 2.6 0a1.3 1.3 0 1 0 -2.6 0z M5 5h9v1H5zm0 10h9v1H5z",
    main: &[
        "M4 3h16a2 2 0 0 1 2 2v3a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-3a2 2 0 0 1 2 -2z",
        "M4 13h16a2 2 0 0 1 2 2v4a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-4a2 2 0 0 1 2 -2z",
    ],
    cutout: &[
        "M16.7 6.5a1.3 1.3 0 1 0 2.6 0a1.3 1.3 0 1 0 -2.6 0z",
        "M16.7 17a1.3 1.3 0 1 0 2.6 0a1.3 1.3 0 1 0 -2.6 0z",
    ],
    overlay: &[("M5 5h9v1H5zm0 10h9v1H5z", 0.38)],
};

const TRASH: Glyph = Glyph {
    path: "M5 7h14l-1 15H6zM3 3h18v3H3zm5-2h8v3H8z M9 10h2v8H9zm4 0h2v8h-2z",
    main: &["M5 7h14l-1 15H6zM3 3h18v3H3zm5-2h8v3H8z"],
    cutout: &["M9 10h2v8H9zm4 0h2v8h-2z"],
    overlay: &[],
};

/// The trash of the recorder, with the lines of the reference: the filled
/// one is the icon of the menus, and WhatsApp draws the recorder with an
/// outline so it does not look like a red alert.
const TRASH_OUTLINE: Glyph = Glyph {
    path: "M4 5h16v2H4z M8 3h8v2H8z M6 7h2v13H6z M16 7h2v13h-2z M6 18h12v2H6z M9 10h2v6H9z M13 10h2v6h-2z",
    main: &[
        "M4 5h16v2H4z",
        "M8 3h8v2H8z",
        "M6 7h2v13H6z",
        "M16 7h2v13h-2z",
        "M6 18h12v2H6z",
        "M9 10h2v6H9z",
        "M13 10h2v6h-2z",
    ],
    cutout: &[],
    overlay: &[],
};

/// The camera of a video call.
///
/// The reference of WhatsApp Desktop draws the video call of a conversation
/// in the header, and the web client has no calls: these two glyphs are the
/// only ones of the set that are not in the origin.
const VIDEO: Glyph = Glyph {
    path: "M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11z",
    main: &[
        "M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11z",
    ],
    cutout: &[],
    overlay: &[],
};

const VOLUME: Glyph = Glyph {
    path: "M4 9h3.5L12 5v14l-4.5-4H4z M15 8.5a4.5 4.5 0 0 1 0 7v-1.6a2.9 2.9 0 0 0 0-3.8z M17.6 6a7.4 7.4 0 0 1 0 12v-1.7a5.7 5.7 0 0 0 0-8.6z",
    main: &[
        "M4 9h3.5L12 5v14l-4.5-4H4z",
        "M15 8.5a4.5 4.5 0 0 1 0 7v-1.6a2.9 2.9 0 0 0 0-3.8z",
        "M17.6 6a7.4 7.4 0 0 1 0 12v-1.7a5.7 5.7 0 0 0 0-8.6z",
    ],
    cutout: &[],
    overlay: &[],
};

const VOLUME_MUTE: Glyph = Glyph {
    path: "M4 9h3.5L12 5v14l-4.5-4H4z M14.9 8.6l1.1-1.1 2.1 2.1 2.1-2.1 1.1 1.1-2.1 2.1 2.1 2.1-1.1 1.1-2.1-2.1-2.1 2.1-1.1-1.1 2.1-2.1z",
    main: &[
        "M4 9h3.5L12 5v14l-4.5-4H4z",
        "M14.9 8.6l1.1-1.1 2.1 2.1 2.1-2.1 1.1 1.1-2.1 2.1 2.1 2.1-1.1 1.1-2.1-2.1-2.1 2.1-1.1-1.1 2.1-2.1z",
    ],
    cutout: &[],
    overlay: &[],
};

const WARNING: Glyph = Glyph {
    path: "M10 2.8a2.3 2.3 0 0 1 4 0l9.1 16A2.1 2.1 0 0 1 21.2 22H2.8a2.1 2.1 0 0 1-1.9-3.2z M10.6 7h2.8l-.4 8h-2zm0 10h2.8v2.5h-2.8z",
    main: &["M10 2.8a2.3 2.3 0 0 1 4 0l9.1 16A2.1 2.1 0 0 1 21.2 22H2.8a2.1 2.1 0 0 1-1.9-3.2z"],
    cutout: &["M10.6 7h2.8l-.4 8h-2zm0 10h2.8v2.5h-2.8z"],
    overlay: &[],
};

/// The icons of Whatfast.
///
/// The name of every variant is the name of the web client, in `PascalCase`:
/// `check-double` becomes [`WhatfastIcon::CheckDouble`] and `new-chat`
/// becomes [`WhatfastIcon::NewChat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhatfastIcon {
    Archive,
    Attach,
    Bell,
    Bot,
    Channels,
    Chat,
    Check,
    CheckDouble,
    Clock,
    Close,
    Copy,
    Document,
    Download,
    Edit,
    Forward,
    Globe,
    Image,
    Maximize,
    Messages,
    Mic,
    Minimize,
    NewChat,
    Palette,
    Pause,
    Phone,
    Pin,
    Play,
    Poll,
    Reply,
    ReplyPrivate,
    Search,
    Send,
    Settings,
    Shield,
    Sidebar,
    Smile,
    Split,
    Status,
    Sticker,
    Storage,
    Trash,
    TrashOutline,
    Video,
    Volume,
    VolumeMute,
    Warning,
}

impl WhatfastIcon {
    /// Every icon of the set.
    pub const ALL: [WhatfastIcon; 46] = [
        WhatfastIcon::Archive,
        WhatfastIcon::Attach,
        WhatfastIcon::Bell,
        WhatfastIcon::Bot,
        WhatfastIcon::Channels,
        WhatfastIcon::Chat,
        WhatfastIcon::Check,
        WhatfastIcon::CheckDouble,
        WhatfastIcon::Clock,
        WhatfastIcon::Close,
        WhatfastIcon::Copy,
        WhatfastIcon::Document,
        WhatfastIcon::Download,
        WhatfastIcon::Edit,
        WhatfastIcon::Forward,
        WhatfastIcon::Globe,
        WhatfastIcon::Image,
        WhatfastIcon::Maximize,
        WhatfastIcon::Messages,
        WhatfastIcon::Mic,
        WhatfastIcon::Minimize,
        WhatfastIcon::NewChat,
        WhatfastIcon::Palette,
        WhatfastIcon::Pause,
        WhatfastIcon::Phone,
        WhatfastIcon::Pin,
        WhatfastIcon::Play,
        WhatfastIcon::Poll,
        WhatfastIcon::Reply,
        WhatfastIcon::ReplyPrivate,
        WhatfastIcon::Search,
        WhatfastIcon::Send,
        WhatfastIcon::Settings,
        WhatfastIcon::Shield,
        WhatfastIcon::Sidebar,
        WhatfastIcon::Smile,
        WhatfastIcon::Split,
        WhatfastIcon::Status,
        WhatfastIcon::Sticker,
        WhatfastIcon::Storage,
        WhatfastIcon::Trash,
        WhatfastIcon::TrashOutline,
        WhatfastIcon::Video,
        WhatfastIcon::Volume,
        WhatfastIcon::VolumeMute,
        WhatfastIcon::Warning,
    ];

    /// The path of the icon: the shapes of the glyph of the web client.
    pub fn path_data(self) -> &'static str {
        self.glyph().path
    }

    /// The icon whose path is the given one.
    pub fn from_path_data(path: &str) -> Option<WhatfastIcon> {
        WhatfastIcon::ALL
            .into_iter()
            .find(|icon| icon.path_data() == path)
    }

    /// The document that the renderer paints.
    ///
    /// The cutouts of the web client are painted with the colour of the
    /// panel, so in the monochrome mask of GPUI they are holes: the mask of
    /// the document paints them black over the white of the shape.
    pub fn svg_document(self) -> String {
        let glyph = self.glyph();
        let mut document = String::with_capacity(1024);
        document.push_str(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">"##);
        document.push_str(
            r##"<defs><mask id="whatfast" maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24">"##,
        );
        document.push_str(r##"<rect x="0" y="0" width="24" height="24" fill="#000"/>"##);
        for shape in glyph.main {
            document.push_str(&format!(r##"<path d="{shape}" fill="#fff"/>"##));
        }
        for shape in glyph.cutout {
            document.push_str(&format!(r##"<path d="{shape}" fill="#000"/>"##));
        }
        document.push_str("</mask></defs>");
        document.push_str(r##"<g mask="url(#whatfast)">"##);
        for shape in glyph.main {
            document.push_str(&format!(r##"<path d="{shape}" fill="#fff"/>"##));
        }
        document.push_str("</g>");
        for (shape, opacity) in glyph.overlay {
            document.push_str(&format!(
                r##"<path d="{shape}" fill="#fff" fill-opacity="{opacity}"/>"##
            ));
        }
        document.push_str("</svg>");
        document
    }

    /// The glyph of the icon.
    fn glyph(self) -> &'static Glyph {
        match self {
            WhatfastIcon::Archive => &ARCHIVE,
            WhatfastIcon::Attach => &ATTACH,
            WhatfastIcon::Bell => &BELL,
            WhatfastIcon::Bot => &BOT,
            WhatfastIcon::Channels => &CHANNELS,
            WhatfastIcon::Chat => &CHAT,
            WhatfastIcon::Check => &CHECK,
            WhatfastIcon::CheckDouble => &CHECK_DOUBLE,
            WhatfastIcon::Clock => &CLOCK,
            WhatfastIcon::Close => &CLOSE,
            WhatfastIcon::Copy => &COPY,
            WhatfastIcon::Document => &DOCUMENT,
            WhatfastIcon::Download => &DOWNLOAD,
            WhatfastIcon::Edit => &EDIT,
            WhatfastIcon::Forward => &FORWARD,
            WhatfastIcon::Globe => &GLOBE,
            WhatfastIcon::Image => &IMAGE,
            WhatfastIcon::Maximize => &MAXIMIZE,
            WhatfastIcon::Messages => &MESSAGES,
            WhatfastIcon::Mic => &MIC,
            WhatfastIcon::Minimize => &MINIMIZE,
            WhatfastIcon::NewChat => &NEW_CHAT,
            WhatfastIcon::Palette => &PALETTE,
            WhatfastIcon::Pause => &PAUSE,
            WhatfastIcon::Phone => &PHONE,
            WhatfastIcon::Pin => &PIN,
            WhatfastIcon::Play => &PLAY,
            WhatfastIcon::Poll => &POLL,
            WhatfastIcon::Reply => &REPLY,
            WhatfastIcon::ReplyPrivate => &REPLY_PRIVATE,
            WhatfastIcon::Search => &SEARCH,
            WhatfastIcon::Send => &SEND,
            WhatfastIcon::Settings => &SETTINGS,
            WhatfastIcon::Shield => &SHIELD,
            WhatfastIcon::Sidebar => &SIDEBAR,
            WhatfastIcon::Smile => &SMILE,
            WhatfastIcon::Split => &SPLIT,
            WhatfastIcon::Status => &STATUS,
            WhatfastIcon::Sticker => &STICKER,
            WhatfastIcon::Storage => &STORAGE,
            WhatfastIcon::Trash => &TRASH,
            WhatfastIcon::TrashOutline => &TRASH_OUTLINE,
            WhatfastIcon::Video => &VIDEO,
            WhatfastIcon::Volume => &VOLUME,
            WhatfastIcon::VolumeMute => &VOLUME_MUTE,
            WhatfastIcon::Warning => &WARNING,
        }
    }
}

impl IconNamed for WhatfastIcon {
    fn path(self) -> SharedString {
        SharedString::new_static(self.path_data())
    }
}

/// The source of the assets of the application: the icons of `gpui_component`
/// plus the icons of Whatfast.
///
/// The path of an icon of Whatfast is the path of its glyph and not the name
/// of a file, so this source builds the document of the icon when the
/// renderer asks for it. The application has to use it instead of the assets
/// of `gpui_component` alone:
///
/// ```ignore
/// gpui_platform::application()
///     .with_assets(WhatfastAssets::new())
///     .run(|cx| { /* ... */ });
/// ```
pub struct WhatfastAssets {
    inner: gpui_component_assets::Assets,
}

impl WhatfastAssets {
    /// Builds the source of the assets.
    pub fn new() -> Self {
        Self {
            inner: gpui_component_assets::Assets::new(""),
        }
    }
}

impl Default for WhatfastAssets {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetSource for WhatfastAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = WhatfastIcon::from_path_data(path) {
            return Ok(Some(Cow::Owned(icon.svg_document().into_bytes())));
        }
        // Los logos de los harnesses: el path es `harness/<id>` y el documento
        // ya esta escrito en el binario. Un harness sin marca contesta que no
        // tiene asset, para que la tarjeta dibuje el robot.
        if let Some(id) = path.strip_prefix("harness/") {
            return Ok(crate::harness_logos::svg(id).map(|logo| Cow::Borrowed(logo.as_bytes())));
        }
        self.inner.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        self.inner.list(path)
    }
}

