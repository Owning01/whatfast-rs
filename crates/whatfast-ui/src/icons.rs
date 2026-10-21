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

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component::{IconName, IconNamed};

/// The shapes of one glyph of the web client.
pub struct Glyph {
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
    main: &["M7.2 12.9 14 6.1a3 3 0 0 1 4.2 4.2l-8.5 8.5a5 5 0 0 1-7.1-7.1l8.1-8.1 1.5 1.5-8.1 8.1a2.9 2.9 0 0 0 4.1 4.1l8.5-8.5a.9.9 0 1 0-1.3-1.3l-6.8 6.8a1 1 0 0 0 1.4 1.4l5.1-5.1 1.5 1.5-5.1 5.1a3.1 3.1 0 0 1-4.3-4.3z"],
    cutout: &[],
    overlay: &[],
};

const BELL: Glyph = Glyph {
    path: "M4 17h16l-2-2.7V9a6 6 0 0 0-12 0v5.3z M9 19h6a3 3 0 0 1-6 0z M8 8.5A4.2 4.2 0 0 1 12 5c1.5 0 2.8.7 3.6 1.7A5 5 0 0 0 8 8.5z",
    main: &["M4 17h16l-2-2.7V9a6 6 0 0 0-12 0v5.3z", "M9 19h6a3 3 0 0 1-6 0z"],
    cutout: &[],
    overlay: &[("M8 8.5A4.2 4.2 0 0 1 12 5c1.5 0 2.8.7 3.6 1.7A5 5 0 0 0 8 8.5z", 0.38)],
};

const CHAT: Glyph = Glyph {
    path: "M3 5.8C3 3.7 5 2 7.4 2h9.2C19 2 21 3.7 21 5.8v7.4c0 2.1-2 3.8-4.4 3.8H11l-5.7 4.2.9-4.5C4.3 16.2 3 14.9 3 13.2z M6.8 5h10.4c.7 0 1.3.4 1.6.9H5.2c.3-.5.9-.9 1.6-.9z",
    main: &["M3 5.8C3 3.7 5 2 7.4 2h9.2C19 2 21 3.7 21 5.8v7.4c0 2.1-2 3.8-4.4 3.8H11l-5.7 4.2.9-4.5C4.3 16.2 3 14.9 3 13.2z"],
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
    overlay: &[("M5.5 8A8.2 8.2 0 0 1 12 4c2.6 0 4.9 1.2 6.4 3A9 9 0 0 0 5.5 8z", 0.38)],
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
        ("M4 7h11a2 2 0 0 1 2 2v11a2 2 0 0 1 -2 2h-11a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2z", 0.72),
        ("M9 4h11v1H9zM4 9h11v1H4z", 0.38),
    ],
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

const MAXIMIZE: Glyph = Glyph {
    path: "M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2z M7 8h10v9h-10z",
    main: &["M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2z"],
    cutout: &["M7 8h10v9h-10z"],
    overlay: &[],
};

const MESSAGES: Glyph = Glyph {
    path: "M2.5 5.5A3.5 3.5 0 0 1 6 2h8a3.5 3.5 0 0 1 3.5 3.5v5A3.5 3.5 0 0 1 14 14H9l-4.6 3.2.8-3.6a3.5 3.5 0 0 1-2.7-3.4z M9 16h6.3l3.8 2.7-.7-3a3.2 3.2 0 0 0 3.1-3.2V9A3.5 3.5 0 0 0 18 5.5v5A4.5 4.5 0 0 1 13.5 15H9z",
    main: &["M2.5 5.5A3.5 3.5 0 0 1 6 2h8a3.5 3.5 0 0 1 3.5 3.5v5A3.5 3.5 0 0 1 14 14H9l-4.6 3.2.8-3.6a3.5 3.5 0 0 1-2.7-3.4z"],
    cutout: &[],
    overlay: &[("M9 16h6.3l3.8 2.7-.7-3a3.2 3.2 0 0 0 3.1-3.2V9A3.5 3.5 0 0 0 18 5.5v5A4.5 4.5 0 0 1 13.5 15H9z", 0.72)],
};

const MIC: Glyph = Glyph {
    path: "M12 2.5a3 3 0 0 1 3 3v6a3 3 0 0 1 -3 3a3 3 0 0 1 -3 -3v-6a3 3 0 0 1 3 -3z M5.5 11.5a6.5 6.5 0 0 0 13 0 M12 18v3.5",
    main: &[
        "M12 2.5a3 3 0 0 1 3 3v6a3 3 0 0 1 -3 3a3 3 0 0 1 -3 -3v-6a3 3 0 0 1 3 -3z",
        "M5.5 11.5a6.5 6.5 0 0 0 13 0",
        "M12 18v3.5",
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
    main: &["M2 5.5A3.5 3.5 0 0 1 5.5 2h9A3.5 3.5 0 0 1 18 5.5v6a3.5 3.5 0 0 1-3.5 3.5H9l-4.8 3.4.8-3.8a3.5 3.5 0 0 1-3-3.5z"],
    cutout: &["M18.5 14v-3h2v3h3v2h-3v3h-2v-3h-3v-2z"],
    overlay: &[],
};

const PALETTE: Glyph = Glyph {
    path: "M12 2C6.2 2 2 5.8 2 10.7 2 15.2 5.5 18 9.2 18h1.1c1 0 1.5.9 1 1.7-.8 1.4.3 2.4 1.9 2.3 5-.5 8.8-4.5 8.8-9.7C22 6.5 17.8 2 12 2z M5.7 9.2a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z M9 6.4a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z M13.5 6.8a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z M16.3 10.5a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0z",
    main: &["M12 2C6.2 2 2 5.8 2 10.7 2 15.2 5.5 18 9.2 18h1.1c1 0 1.5.9 1 1.7-.8 1.4.3 2.4 1.9 2.3 5-.5 8.8-4.5 8.8-9.7C22 6.5 17.8 2 12 2z"],
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

const POLL: Glyph = Glyph {
    path: "M5.5 3h13a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-13a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z M7 7h2v8H7zm4 3h2v5h-2zm4-5h2v10h-2zm-8 11h10v2H7z",
    main: &["M5.5 3h13a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-13a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z"],
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
    overlay: &[("M6.5 8.8A4.4 4.4 0 0 1 10.4 6c1.2 0 2.3.5 3.1 1.2a5 5 0 0 0-7 1.6z", 0.38)],
};

const SEND: Glyph = Glyph {
    path: "M2 3.2 22 12 2 20.8l2.2-7.3L15 12 4.2 10.5z m4 5.5 14.2 6.2-13.6-3z",
    main: &["M2 3.2 22 12 2 20.8l2.2-7.3L15 12 4.2 10.5z"],
    cutout: &[],
    overlay: &[("m4 5.5 14.2 6.2-13.6-3z", 0.38)],
};

const SETTINGS: Glyph = Glyph {
    path: "m10.2 1.5-.7 2.3-1.6.7-2.2-1.1-2.3 2.3 1.1 2.2-.7 1.6-2.3.7v3.3l2.3.7.7 1.6L3.4 18l2.3 2.3 2.2-1.1 1.6.7.7 2.3h3.3l.7-2.3 1.6-.7 2.2 1.1 2.3-2.3-1.1-2.2.7-1.6 2.3-.7v-3.3l-2.3-.7-.7-1.6 1.1-2.2L18 3.4l-2.2 1.1-1.6-.7-.7-2.3z M8.4 11.8a3.4 3.4 0 1 0 6.8 0a3.4 3.4 0 1 0 -6.8 0z M10.5 3h2l.4 1.5h-2.8z",
    main: &["m10.2 1.5-.7 2.3-1.6.7-2.2-1.1-2.3 2.3 1.1 2.2-.7 1.6-2.3.7v3.3l2.3.7.7 1.6L3.4 18l2.3 2.3 2.2-1.1 1.6.7.7 2.3h3.3l.7-2.3 1.6-.7 2.2 1.1 2.3-2.3-1.1-2.2.7-1.6 2.3-.7v-3.3l-2.3-.7-.7-1.6 1.1-2.2L18 3.4l-2.2 1.1-1.6-.7-.7-2.3z"],
    cutout: &["M8.4 11.8a3.4 3.4 0 1 0 6.8 0a3.4 3.4 0 1 0 -6.8 0z"],
    overlay: &[("M10.5 3h2l.4 1.5h-2.8z", 0.38)],
};

const SHIELD: Glyph = Glyph {
    path: "M12 1.8 21 5v6.2c0 5.2-3.7 9.4-9 11-5.3-1.6-9-5.8-9-11V5z m8.4 11.6 2.1 2.1 4.9-5 1.4 1.5-6.3 6.3L7 13z M12 4 5.4 6.3v4.8c0 .5 0 1 .2 1.5C6.2 8.6 8.6 6 12 4z",
    main: &["M12 1.8 21 5v6.2c0 5.2-3.7 9.4-9 11-5.3-1.6-9-5.8-9-11V5z"],
    cutout: &["m8.4 11.6 2.1 2.1 4.9-5 1.4 1.5-6.3 6.3L7 13z"],
    overlay: &[("M12 4 5.4 6.3v4.8c0 .5 0 1 .2 1.5C6.2 8.6 8.6 6 12 4z", 0.38)],
};

const SIDEBAR: Glyph = Glyph {
    path: "M4.5 3h15a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-15a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z M8 5h2v14H8z M4 5h3v1H4zm8 0h8v1h-8z",
    main: &["M4.5 3h15a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-15a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z"],
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
    overlay: &[("M5.5 7.4A8.5 8.5 0 0 1 12 4c2.5 0 4.8 1.1 6.3 2.8A9.1 9.1 0 0 0 5.5 7.4z", 0.38)],
};

const SPLIT: Glyph = Glyph {
    path: "M4.5 3h15a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-15a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z M11 4h2v16h-2z M4 5h6v1H4zm10 0h6v1h-6z",
    main: &["M4.5 3h15a2.5 2.5 0 0 1 2.5 2.5v13a2.5 2.5 0 0 1 -2.5 2.5h-15a2.5 2.5 0 0 1 -2.5 -2.5v-13a2.5 2.5 0 0 1 2.5 -2.5z"],
    cutout: &["M11 4h2v16h-2z"],
    overlay: &[("M4 5h6v1H4zm10 0h6v1h-6z", 0.38)],
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
    Chat,
    Check,
    CheckDouble,
    Clock,
    Close,
    Copy,
    Download,
    Edit,
    Forward,
    Globe,
    Maximize,
    Messages,
    Mic,
    Minimize,
    NewChat,
    Palette,
    Pin,
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
    Sticker,
    Storage,
    Trash,
    Warning,
}

impl WhatfastIcon {
    /// Every icon of the set.
    pub const ALL: [WhatfastIcon; 34] = [
        WhatfastIcon::Archive,
        WhatfastIcon::Attach,
        WhatfastIcon::Bell,
        WhatfastIcon::Chat,
        WhatfastIcon::Check,
        WhatfastIcon::CheckDouble,
        WhatfastIcon::Clock,
        WhatfastIcon::Close,
        WhatfastIcon::Copy,
        WhatfastIcon::Download,
        WhatfastIcon::Edit,
        WhatfastIcon::Forward,
        WhatfastIcon::Globe,
        WhatfastIcon::Maximize,
        WhatfastIcon::Messages,
        WhatfastIcon::Mic,
        WhatfastIcon::Minimize,
        WhatfastIcon::NewChat,
        WhatfastIcon::Palette,
        WhatfastIcon::Pin,
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
        WhatfastIcon::Sticker,
        WhatfastIcon::Storage,
        WhatfastIcon::Trash,
        WhatfastIcon::Warning,
    ];

    /// The name of the icon in the web client.
    pub fn name(self) -> &'static str {
        match self {
            WhatfastIcon::Archive => "archive",
            WhatfastIcon::Attach => "attach",
            WhatfastIcon::Bell => "bell",
            WhatfastIcon::Chat => "chat",
            WhatfastIcon::Check => "check",
            WhatfastIcon::CheckDouble => "check-double",
            WhatfastIcon::Clock => "clock",
            WhatfastIcon::Close => "close",
            WhatfastIcon::Copy => "copy",
            WhatfastIcon::Download => "download",
            WhatfastIcon::Edit => "edit",
            WhatfastIcon::Forward => "forward",
            WhatfastIcon::Globe => "globe",
            WhatfastIcon::Maximize => "maximize",
            WhatfastIcon::Messages => "messages",
            WhatfastIcon::Mic => "mic",
            WhatfastIcon::Minimize => "minimize",
            WhatfastIcon::NewChat => "new-chat",
            WhatfastIcon::Palette => "palette",
            WhatfastIcon::Pin => "pin",
            WhatfastIcon::Poll => "poll",
            WhatfastIcon::Reply => "reply",
            WhatfastIcon::ReplyPrivate => "reply-private",
            WhatfastIcon::Search => "search",
            WhatfastIcon::Send => "send",
            WhatfastIcon::Settings => "settings",
            WhatfastIcon::Shield => "shield",
            WhatfastIcon::Sidebar => "sidebar",
            WhatfastIcon::Smile => "smile",
            WhatfastIcon::Split => "split",
            WhatfastIcon::Sticker => "sticker",
            WhatfastIcon::Storage => "storage",
            WhatfastIcon::Trash => "trash",
            WhatfastIcon::Warning => "warning",
        }
    }

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
            WhatfastIcon::Chat => &CHAT,
            WhatfastIcon::Check => &CHECK,
            WhatfastIcon::CheckDouble => &CHECK_DOUBLE,
            WhatfastIcon::Clock => &CLOCK,
            WhatfastIcon::Close => &CLOSE,
            WhatfastIcon::Copy => &COPY,
            WhatfastIcon::Download => &DOWNLOAD,
            WhatfastIcon::Edit => &EDIT,
            WhatfastIcon::Forward => &FORWARD,
            WhatfastIcon::Globe => &GLOBE,
            WhatfastIcon::Maximize => &MAXIMIZE,
            WhatfastIcon::Messages => &MESSAGES,
            WhatfastIcon::Mic => &MIC,
            WhatfastIcon::Minimize => &MINIMIZE,
            WhatfastIcon::NewChat => &NEW_CHAT,
            WhatfastIcon::Palette => &PALETTE,
            WhatfastIcon::Pin => &PIN,
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
            WhatfastIcon::Sticker => &STICKER,
            WhatfastIcon::Storage => &STORAGE,
            WhatfastIcon::Trash => &TRASH,
            WhatfastIcon::Warning => &WARNING,
        }
    }
}

impl IconNamed for WhatfastIcon {
    fn path(self) -> SharedString {
        SharedString::new_static(self.path_data())
    }
}

/// The icon of the button that switches between the light and the dark theme.
///
/// port: the web client keeps the same rule in its panel of appearance: the
/// sun is the way back to the light theme and the moon the way back to the
/// dark one. The native client draws that button with the outline set of
/// `gpui_component`, because the set of Whatfast has no sun.
pub fn theme_toggle_icon(dark: bool) -> IconName {
    if dark { IconName::Sun } else { IconName::Moon }
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
        self.inner.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        self.inner.list(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The number of arguments of every command of a path.
    fn arguments(command: char) -> Option<usize> {
        match command {
            'M' | 'm' | 'L' | 'l' | 'T' | 't' => Some(2),
            'H' | 'h' | 'V' | 'v' => Some(1),
            'C' | 'c' => Some(6),
            'S' | 's' | 'Q' | 'q' => Some(4),
            'A' | 'a' => Some(7),
            'Z' | 'z' => Some(0),
            _ => None,
        }
    }

    /// True when the path is a valid path of SVG: it starts with a move, every
    /// command is known and has all its arguments, and it ends with a
    /// command that closes the path.
    fn is_valid_path(path: &str) -> bool {
        let path = path.trim();
        if path.is_empty() {
            return false;
        }
        let mut command = None;
        let mut numbers = 0usize;
        let mut closed = false;
        for piece in path.split(|character: char| {
            character.is_ascii_alphabetic() || character.is_ascii_digit()
                || matches!(character, '.' | '-' | '+' | 'e' | 'E')
        }) {
            if piece.is_empty() {
                continue;
            }
            if piece.chars().next().is_some_and(|first| first.is_ascii_alphabetic()) {
                if let Some(previous) = command {
                    let expected = arguments(previous).unwrap_or(0);
                    if numbers % expected != 0 {
                        return false;
                    }
                    closed = matches!(previous, 'Z' | 'z');
                }
                let first = piece.chars().next().unwrap_or(' ');
                if arguments(first).is_none() {
                    return false;
                }
                command = Some(first);
                numbers = 0;
                continue;
            }
            let value: f64 = match piece.parse() {
                Ok(value) => value,
                Err(_) => return false,
            };
            if !value.is_finite() {
                return false;
            }
            numbers += 1;
        }
        let Some(last) = command else {
            return false;
        };
        let expected = arguments(last).unwrap_or(0);
        if numbers % expected != 0 {
            return false;
        }
        // The path has to start with a move and to end with a command that
        // leaves the point of the path on a shape.
        let starts_with_move = path
            .chars()
            .next()
            .is_some_and(|first| matches!(first, 'M' | 'm'));
        starts_with_move && closed
    }

    #[test]
    fn every_icon_has_a_path() {
        for icon in WhatfastIcon::ALL {
            assert!(
                !icon.path_data().is_empty(),
                "the icon {} has no path",
                icon.name()
            );
        }
    }

    #[test]
    fn every_icon_has_a_valid_path() {
        for icon in WhatfastIcon::ALL {
            assert!(
                is_valid_path(icon.path_data()),
                "the path of {} is not a valid path of SVG: {}",
                icon.name(),
                icon.path_data()
            );
        }
    }

    #[test]
    fn every_shape_of_the_glyph_has_a_valid_path() {
        for icon in WhatfastIcon::ALL {
            let glyph = icon.glyph();
            for shape in glyph.main.iter().chain(glyph.cutout.iter()) {
                assert!(
                    is_valid_path(shape),
                    "a shape of {} is not a valid path of SVG: {shape}",
                    icon.name()
                );
            }
            for (shape, opacity) in glyph.overlay {
                assert!(
                    is_valid_path(shape),
                    "a shape of {} is not a valid path of SVG: {shape}",
                    icon.name()
                );
                assert!(
                    *opacity > 0.0 && *opacity <= 1.0,
                    "the opacity of {} is out of range: {opacity}",
                    icon.name()
                );
            }
        }
    }

    #[test]
    fn the_path_of_the_icon_is_the_join_of_its_shapes() {
        for icon in WhatfastIcon::ALL {
            let glyph = icon.glyph();
            let mut shapes: Vec<&str> = glyph.main.to_vec();
            shapes.extend_from_slice(glyph.cutout);
            shapes.extend(glyph.overlay.iter().map(|(shape, _)| *shape));
            let joined = shapes.join(" ");
            assert_eq!(
                icon.path_data(),
                joined,
                "the path of {} is not the join of its shapes",
                icon.name()
            );
        }
    }

    #[test]
    fn the_names_of_the_icons_are_the_ones_of_the_web_client() {
        let names: Vec<&str> = WhatfastIcon::ALL.iter().map(|icon| icon.name()).collect();
        assert_eq!(
            names,
            vec![
                "archive",
                "attach",
                "bell",
                "chat",
                "check",
                "check-double",
                "clock",
                "close",
                "copy",
                "download",
                "edit",
                "forward",
                "globe",
                "maximize",
                "messages",
                "mic",
                "minimize",
                "new-chat",
                "palette",
                "pin",
                "poll",
                "reply",
                "reply-private",
                "search",
                "send",
                "settings",
                "shield",
                "sidebar",
                "smile",
                "split",
                "sticker",
                "storage",
                "trash",
                "warning",
            ]
        );
    }

    #[test]
    fn the_names_of_the_icons_are_different() {
        let mut names: Vec<&str> = WhatfastIcon::ALL.iter().map(|icon| icon.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two icons share the same name");
    }

    #[test]
    fn the_paths_of_the_icons_are_different() {
        let mut paths: Vec<&str> = WhatfastIcon::ALL
            .iter()
            .map(|icon| icon.path_data())
            .collect();
        paths.sort_unstable();
        let count = paths.len();
        paths.dedup();
        assert_eq!(paths.len(), count, "two icons share the same path");
    }

    #[test]
    fn the_icon_of_a_path_is_the_one_that_has_it() {
        for icon in WhatfastIcon::ALL {
            assert_eq!(
                WhatfastIcon::from_path_data(icon.path_data()),
                Some(icon),
                "the path of {} does not find its own icon",
                icon.name()
            );
        }
        assert_eq!(WhatfastIcon::from_path_data("icons/close.svg"), None);
        assert_eq!(WhatfastIcon::from_path_data(""), None);
    }

    #[test]
    fn the_trait_gives_the_path_of_the_icon() {
        for icon in WhatfastIcon::ALL {
            assert_eq!(
                IconNamed::path(icon).as_str(),
                icon.path_data(),
                "the trait does not give the path of {}",
                icon.name()
            );
        }
    }

    #[test]
    fn the_document_of_the_icon_has_the_shapes_and_the_holes() {
        for icon in WhatfastIcon::ALL {
            let document = icon.svg_document();
            assert!(document.starts_with("<svg "), "the document of {} is not an SVG", icon.name());
            assert!(document.ends_with("</svg>"), "the document of {} is not closed", icon.name());
            assert!(
                document.contains(r#"viewBox="0 0 24 24""#),
                "the document of {} has no view box",
                icon.name()
            );
            let glyph = icon.glyph();
            for shape in glyph.main {
                assert!(
                    document.contains(&format!(r##"<path d="{shape}" fill="#fff"/>"##)),
                    "the document of {} does not have the shape {shape}",
                    icon.name()
                );
            }
            for shape in glyph.cutout {
                assert!(
                    document.contains(&format!(r##"<path d="{shape}" fill="#000"/>"##)),
                    "the document of {} does not have the hole {shape}",
                    icon.name()
                );
            }
        }
    }

    #[test]
    fn the_source_of_the_assets_serves_the_icons_of_whatfast() {
        let assets = WhatfastAssets::new();
        for icon in WhatfastIcon::ALL {
            let loaded = assets.load(icon.path_data());
            let loaded = match loaded {
                Ok(loaded) => loaded,
                Err(error) => panic!("the icon {} could not be loaded: {error}", icon.name()),
            };
            let bytes = match loaded {
                Some(bytes) => bytes,
                None => panic!("the icon {} was not served", icon.name()),
            };
            let document = String::from_utf8(bytes.to_vec())
                .unwrap_or_else(|_| panic!("the icon {} is not a document", icon.name()));
            assert_eq!(document, icon.svg_document());
        }
    }

    #[test]
    fn the_source_of_the_assets_serves_the_icons_of_the_component() {
        let assets = WhatfastAssets::new();
        assert!(
            assets.load("icons/close.svg").is_ok(),
            "the icons of gpui_component have to keep working"
        );
    }

    #[test]
    fn the_icon_of_the_theme_follows_the_appearance() {
        // `IconName` is generated and does not implement `PartialEq`, so the
        // variants are matched instead of compared.
        assert!(matches!(theme_toggle_icon(true), IconName::Sun));
        assert!(matches!(theme_toggle_icon(false), IconName::Moon));
    }
}
