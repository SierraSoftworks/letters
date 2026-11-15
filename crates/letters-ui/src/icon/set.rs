//! The icons the editor window uses, copied from yewi's bundled Lucide set (ISC licence, https://lucide.dev).
//! Add one by copying its constant from `yewi/src/icons/lucide.rs`.

use super::Icon;

/// `arrow-left`
pub const ARROW_LEFT: Icon = Icon::new("M12 19l-7-7 7-7M19 12H5");
/// `arrow-left-right`
pub const ARROW_LEFT_RIGHT: Icon = Icon::new("M8 3 4 7l4 4M4 7h16M16 21l4-4-4-4M20 17H4");
/// `baseline`
pub const BASELINE: Icon = Icon::new("M4 20h16M6 16l6-12 6 12M8 12h8");
/// `bold`
pub const BOLD: Icon = Icon::new("M6 12h9a4 4 0 010 8H7a1 1 0 01-1-1V5a1 1 0 011-1h7a4 4 0 010 8");
/// `check`
pub const CHECK: Icon = Icon::new("M20 6 9 17l-5-5");
/// `chevron-down`
pub const CHEVRON_DOWN: Icon = Icon::new("M6 9l6 6 6-6");
/// `chevron-left`
pub const CHEVRON_LEFT: Icon = Icon::new("M15 18l-6-6 6-6");
/// `chevron-right`
pub const CHEVRON_RIGHT: Icon = Icon::new("M9 18l6-6-6-6");
/// `chevron-up`
pub const CHEVRON_UP: Icon = Icon::new("M18 15l-6-6-6 6");
/// `clipboard-paste`
pub const CLIPBOARD_PASTE: Icon = Icon::new("M11 14h10M16 4h2a2 2 0 0 1 2 2v1.344M17 18l4-4-4-4M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 1.793-1.113M9 2h6a1 1 0 0 1 1 1v2a1 1 0 0 1 -1 1h-6a1 1 0 0 1 -1 -1v-2a1 1 0 0 1 1 -1z");
/// `cloud`
pub const CLOUD: Icon = Icon::new("M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z");
/// `cloud-alert`
pub const CLOUD_ALERT: Icon = Icon::new("M12 12v4M12 20h.01M8.128 16.949A7 7 0 1 1 15.71 8h1.79a1 1 0 0 1 0 9h-1.642");
/// `cloud-check`
pub const CLOUD_CHECK: Icon = Icon::new("M17 15l-5.5 5.5L9 18M5.516 16.07A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 3.501 7.327");
/// `cloud-download`
pub const CLOUD_DOWNLOAD: Icon = Icon::new("M12 13v8l-4-4M12 21l4-4M4.393 15.269A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.436 8.284");
/// `cloud-off`
pub const CLOUD_OFF: Icon = Icon::new("M10.94 5.274A7 7 0 0 1 15.71 10h1.79a4.5 4.5 0 0 1 4.222 6.057M18.796 18.81A4.5 4.5 0 0 1 17.5 19H9A7 7 0 0 1 5.79 5.78M2 2l20 20");
/// `code`
pub const CODE: Icon = Icon::new("M16 18l6-6-6-6M8 6l-6 6 6 6");
/// `code-xml`
pub const CODE_XML: Icon = Icon::new("M18 16l4-4-4-4M6 8l-4 4 4 4M14.5 4l-5 16");
/// `copy`
pub const COPY: Icon = Icon::new("M10 8h10a2 2 0 0 1 2 2v10a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2v-10a2 2 0 0 1 2 -2zM4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2");
/// `crown`
pub const CROWN: Icon = Icon::new("M11.562 3.266a.5.5 0 0 1 .876 0L15.39 8.87a1 1 0 0 0 1.516.294L21.183 5.5a.5.5 0 0 1 .798.519l-2.834 10.246a1 1 0 0 1-.956.734H5.81a1 1 0 0 1-.957-.734L2.02 6.02a.5.5 0 0 1 .798-.519l4.276 3.664a1 1 0 0 0 1.516-.294zM5 21h14");
/// `download`
pub const DOWNLOAD: Icon = Icon::new("M12 15V3M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5");
/// `ellipsis`
pub const ELLIPSIS: Icon = Icon::new("M11 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0M18 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0M4 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0");
/// `external-link`
pub const EXTERNAL_LINK: Icon = Icon::new("M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6");
/// `eye`
pub const EYE: Icon = Icon::new("M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0M9 12a3 3 0 1 0 6 0a3 3 0 1 0 -6 0");
/// `file-down`
pub const FILE_DOWN: Icon = Icon::new("M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2zM14 2v5a1 1 0 0 0 1 1h5M12 18v-6M9 15l3 3 3-3");
/// `file-input`
pub const FILE_INPUT: Icon = Icon::new("M4 11V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.706.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-1M14 2v5a1 1 0 0 0 1 1h5M2 15h10M9 18l3-3-3-3");
/// `file-text`
pub const FILE_TEXT: Icon = Icon::new("M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2zM14 2v5a1 1 0 0 0 1 1h5M10 9H8M16 13H8M16 17H8");
/// `file-up`
pub const FILE_UP: Icon = Icon::new("M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2zM14 2v5a1 1 0 0 0 1 1h5M12 12v6M15 15l-3-3-3 3");
/// `folder`
pub const FOLDER: Icon = Icon::new("M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z");
/// `folder-open`
pub const FOLDER_OPEN: Icon = Icon::new("M6 14l1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2");
/// `folder-plus`
pub const FOLDER_PLUS: Icon = Icon::new("M12 10v6M9 13h6M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z");
/// `folder-search`
pub const FOLDER_SEARCH: Icon = Icon::new("M10.7 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v4.1M21 21l-1.9-1.9M14 17a3 3 0 1 0 6 0a3 3 0 1 0 -6 0");
/// `git-merge`
pub const GIT_MERGE: Icon = Icon::new("M15 18a3 3 0 1 0 6 0a3 3 0 1 0 -6 0M3 6a3 3 0 1 0 6 0a3 3 0 1 0 -6 0M6 21V9a9 9 0 0 0 9 9");
/// `heading-1`
pub const HEADING_1: Icon = Icon::new("M4 12h8M4 18V6M12 18V6M17 12l3-2v8");
/// `heading-2`
pub const HEADING_2: Icon = Icon::new("M4 12h8M4 18V6M12 18V6M21 18h-4c0-4 4-3 4-6 0-1.5-2-2.5-4-1");
/// `heading-3`
pub const HEADING_3: Icon = Icon::new("M4 12h8M4 18V6M12 18V6M17.5 10.5c1.7-1 3.5 0 3.5 1.5a2 2 0 0 1-2 2M17 17.5c2 1.5 4 .3 4-1.5a2 2 0 0 0-2-2");
/// `highlighter`
pub const HIGHLIGHTER: Icon = Icon::new("M9 11l-6 6v3h9l3-3M22 12l-4.6 4.6a2 2 0 0 1-2.8 0l-5.2-5.2a2 2 0 0 1 0-2.8L14 4");
/// `history`
pub const HISTORY: Icon = Icon::new("M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8M3 3v5h5M12 7v5l4 2");
/// `image`
pub const IMAGE: Icon = Icon::new("M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2zM7 9a2 2 0 1 0 4 0a2 2 0 1 0 -4 0M21 15l-3.086-3.086a2 2 0 0 0-2.828 0L6 21");
/// `info`
pub const INFO: Icon = Icon::new("M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0M12 16v-4M12 8h.01");
/// `italic`
pub const ITALIC: Icon = Icon::new("M19 4L10 4M14 20L5 20M15 4L9 20");
/// `line-height`
pub const LINE_HEIGHT: Icon = Icon::new("M15 14H9M21 21H3M21 3H3M8 16l3.453-7.648a.6.6 0 011.094.001L16 16");
/// `link`
pub const LINK: Icon = Icon::new("M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71");
/// `list`
pub const LIST: Icon = Icon::new("M3 5h.01M3 12h.01M3 19h.01M8 5h13M8 12h13M8 19h13");
/// `list-checks`
pub const LIST_CHECKS: Icon = Icon::new("M13 5h8M13 12h8M13 19h8M3 17l2 2 4-4M3 7l2 2 4-4");
/// `list-ordered`
pub const LIST_ORDERED: Icon = Icon::new("M11 5h10M11 12h10M11 19h10M4 4h1v5M4 9h2M6.5 20H3.4c0-1 2.6-1.925 2.6-3.5a1.5 1.5 0 0 0-2.6-1.02");
/// `list-tree`
pub const LIST_TREE: Icon = Icon::new("M8 5h13M13 12h8M13 19h8M3 10a2 2 0 0 0 2 2h3M3 5v12a2 2 0 0 0 2 2h3");
/// `message-square`
pub const MESSAGE_SQUARE: Icon = Icon::new("M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z");
/// `message-square-plus`
pub const MESSAGE_SQUARE_PLUS: Icon = Icon::new("M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2zM12 8v6M9 11h6");
/// `minus`
pub const MINUS: Icon = Icon::new("M5 12h14");
/// `mouse-pointer-click`
pub const MOUSE_POINTER_CLICK: Icon = Icon::new("M14 4.1 12 6M5.1 8l-2.9-.8M6 12l-1.9 2M7.2 2.2 8 5.1M9.037 9.69a.498.498 0 0 1 .653-.653l11 4.5a.5.5 0 0 1-.074.949l-4.349 1.041a1 1 0 0 0-.74.739l-1.04 4.35a.5.5 0 0 1-.95.074z");
/// `palette`
pub const PALETTE: Icon = Icon::new("M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8zM13 6.5a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0 -1 0M17 10.5a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0 -1 0M6 12.5a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0 -1 0M8 7.5a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0 -1 0");
/// `pencil`
pub const PENCIL: Icon = Icon::new("M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497zM15 5l4 4");
/// `pilcrow`
pub const PILCROW: Icon = Icon::new("M13 4v16M17 4v16M19 4H9.5a4.5 4.5 0 0 0 0 9H13");
/// `plus`
pub const PLUS: Icon = Icon::new("M5 12h14M12 5v14");
/// `printer`
pub const PRINTER: Icon = Icon::new("M6 18H4a2 2 0 0 1-2-2v-5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2h-2M6 9V3a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v6M7 14h10a1 1 0 0 1 1 1v6a1 1 0 0 1 -1 1h-10a1 1 0 0 1 -1 -1v-6a1 1 0 0 1 1 -1z");
/// `redo-2`
pub const REDO_2: Icon = Icon::new("M15 14l5-5-5-5M20 9H9.5A5.5 5.5 0 0 0 4 14.5A5.5 5.5 0 0 0 9.5 20H13");
/// `refresh-cw`
pub const REFRESH_CW: Icon = Icon::new("M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8M21 3v5h-5M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16M8 16H3v5");
/// `rotate-ccw`
pub const ROTATE_CCW: Icon = Icon::new("M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8M3 3v5h5");
/// `save`
pub const SAVE: Icon = Icon::new(
    "M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7M7 3v4a1 1 0 0 0 1 1h7",
);
/// `scissors`
pub const SCISSORS: Icon = Icon::new("M3 6a3 3 0 1 0 6 0a3 3 0 1 0 -6 0M8.12 8.12 12 12M20 4 8.12 15.88M3 18a3 3 0 1 0 6 0a3 3 0 1 0 -6 0M14.8 14.8 20 20");
/// `search`
pub const SEARCH: Icon = Icon::new("M21 21l-4.34-4.34M3 11a8 8 0 1 0 16 0a8 8 0 1 0 -16 0");
/// `send`
pub const SEND: Icon = Icon::new("M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11zM21.854 2.147l-10.94 10.939");
/// `separator-horizontal`
pub const SEPARATOR_HORIZONTAL: Icon = Icon::new("M16 16l-4 4-4-4M3 12h18M8 8l4-4 4 4");
/// `settings`
pub const SETTINGS: Icon = Icon::new("M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915M9 12a3 3 0 1 0 6 0a3 3 0 1 0 -6 0");
/// `shield`
pub const SHIELD: Icon = Icon::new("M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z");
/// `sigma`
pub const SIGMA: Icon = Icon::new("M18 7V5a1 1 0 0 0-1-1H6.5a.5.5 0 0 0-.4.8l4.5 6a2 2 0 0 1 0 2.4l-4.5 6a.5.5 0 0 0 .4.8H17a1 1 0 0 0 1-1v-2");
/// `square`
pub const SQUARE: Icon = Icon::new("M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z");
/// `strikethrough`
pub const STRIKETHROUGH: Icon = Icon::new("M16 4H9a3 3 0 0 0-2.83 4M14 12a4 4 0 0 1 0 8H6M4 12L20 12");
/// `table`
pub const TABLE: Icon = Icon::new("M12 3v18M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2zM3 9h18M3 15h18");
/// `text-align-center`
pub const TEXT_ALIGN_CENTER: Icon = Icon::new("M21 5H3M17 12H7M19 19H5");
/// `text-align-end`
pub const TEXT_ALIGN_END: Icon = Icon::new("M21 5H3M21 12H9M21 19H7");
/// `text-align-justify`
pub const TEXT_ALIGN_JUSTIFY: Icon = Icon::new("M3 5h18M3 12h18M3 19h18");
/// `text-align-start`
pub const TEXT_ALIGN_START: Icon = Icon::new("M21 5H3M15 12H3M17 19H3");
/// `tag`
pub const TAG: Icon = Icon::new(
    "M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42zM7 7.5a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0 -1 0",
);
/// `text-quote`
pub const TEXT_QUOTE: Icon = Icon::new("M17 5H3M21 12H8M21 19H8M3 12v7");
/// `trash-2`
pub const TRASH_2: Icon = Icon::new("M10 11v6M14 11v6M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2");
/// `type`
pub const TYPE: Icon = Icon::new("M12 4v16M4 7V5a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v2M9 20h6");
/// `underline`
pub const UNDERLINE: Icon = Icon::new("M6 4v6a6 6 0 0 0 12 0V4M4 20L20 20");
/// `undo-2`
pub const UNDO_2: Icon = Icon::new("M9 14 4 9l5-5M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11");
/// `upload`
pub const UPLOAD: Icon = Icon::new("M12 3v12M17 8l-5-5-5 5M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4");
/// `users`
pub const USERS: Icon = Icon::new("M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2M16 3.128a4 4 0 0 1 0 7.744M22 21v-2a4 4 0 0 0-3-3.87M5 7a4 4 0 1 0 8 0a4 4 0 1 0 -8 0");
/// `x`
pub const X: Icon = Icon::new("M18 6 6 18M6 6l12 12");
/// `arrow-right`
pub const ARROW_RIGHT: Icon = Icon::new("M5 12h14M12 5l7 7-7 7");
/// indent: lines that a chevron pushes in
pub const INDENT: Icon = Icon::new("M21 5H11M21 12H11M21 19H11M3 8l4 4-4 4");
/// outdent: lines that a chevron pulls out
pub const OUTDENT: Icon = Icon::new("M21 5H11M21 12H11M21 19H11M7 8l-4 4 4 4");
/// `scan-text`
pub const SCAN_TEXT: Icon = Icon::new("M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2M7 8h8M7 12h10M7 16h6");
/// A table's row with room above it, for what is added there. Drawn for Letters, as are those after it.
pub const ROW_LOW: Icon = Icon::new("M4.5 14H19.5a1.5 1.5 0 0 1 1.5 1.5V18.5a1.5 1.5 0 0 1 -1.5 1.5H4.5a1.5 1.5 0 0 1 -1.5 -1.5V15.5a1.5 1.5 0 0 1 1.5 -1.5z");
/// A row with room below it.
pub const ROW_HIGH: Icon = Icon::new("M4.5 4H19.5a1.5 1.5 0 0 1 1.5 1.5V8.5a1.5 1.5 0 0 1 -1.5 1.5H4.5a1.5 1.5 0 0 1 -1.5 -1.5V5.5a1.5 1.5 0 0 1 1.5 -1.5z");
/// A row that moves up.
pub const ROW_UP: Icon = Icon::new("M4.5 14H19.5a1.5 1.5 0 0 1 1.5 1.5V18.5a1.5 1.5 0 0 1 -1.5 1.5H4.5a1.5 1.5 0 0 1 -1.5 -1.5V15.5a1.5 1.5 0 0 1 1.5 -1.5zM12 10.5V3M8.5 6.5 12 3l3.5 3.5");
/// A row that moves down.
pub const ROW_DOWN: Icon = Icon::new("M4.5 4H19.5a1.5 1.5 0 0 1 1.5 1.5V8.5a1.5 1.5 0 0 1 -1.5 1.5H4.5a1.5 1.5 0 0 1 -1.5 -1.5V5.5a1.5 1.5 0 0 1 1.5 -1.5zM12 13.5V21M8.5 17.5 12 21l3.5-3.5");
/// A row with room after it, for the cross that takes it away.
pub const ROW_SHORT: Icon = Icon::new("M3.5 8.5H12.5a1.5 1.5 0 0 1 1.5 1.5V14.0a1.5 1.5 0 0 1 -1.5 1.5H3.5a1.5 1.5 0 0 1 -1.5 -1.5V10.0a1.5 1.5 0 0 1 1.5 -1.5z");
/// A column with room to its left.
pub const COLUMN_RIGHT: Icon = Icon::new("M15.5 3H18.5a1.5 1.5 0 0 1 1.5 1.5V19.5a1.5 1.5 0 0 1 -1.5 1.5H15.5a1.5 1.5 0 0 1 -1.5 -1.5V4.5a1.5 1.5 0 0 1 1.5 -1.5z");
/// A column with room to its right.
pub const COLUMN_LEFT: Icon = Icon::new("M5.5 3H8.5a1.5 1.5 0 0 1 1.5 1.5V19.5a1.5 1.5 0 0 1 -1.5 1.5H5.5a1.5 1.5 0 0 1 -1.5 -1.5V4.5a1.5 1.5 0 0 1 1.5 -1.5z");
/// A column that moves left.
pub const COLUMN_TO_LEFT: Icon = Icon::new("M15.5 3H18.5a1.5 1.5 0 0 1 1.5 1.5V19.5a1.5 1.5 0 0 1 -1.5 1.5H15.5a1.5 1.5 0 0 1 -1.5 -1.5V4.5a1.5 1.5 0 0 1 1.5 -1.5zM10.5 12H3M6.5 8.5 3 12l3.5 3.5");
/// A column that moves right.
pub const COLUMN_TO_RIGHT: Icon = Icon::new("M5.5 3H8.5a1.5 1.5 0 0 1 1.5 1.5V19.5a1.5 1.5 0 0 1 -1.5 1.5H5.5a1.5 1.5 0 0 1 -1.5 -1.5V4.5a1.5 1.5 0 0 1 1.5 -1.5zM13.5 12H21M17.5 8.5 21 12l-3.5 3.5");
/// A column with room under it.
pub const COLUMN_SHORT: Icon = Icon::new("M10.0 2H14.0a1.5 1.5 0 0 1 1.5 1.5V12.5a1.5 1.5 0 0 1 -1.5 1.5H10.0a1.5 1.5 0 0 1 -1.5 -1.5V3.5a1.5 1.5 0 0 1 1.5 -1.5z");
/// A plus in the room above a row.
pub const MARK_PLUS_TOP: Icon = Icon::new("M12 2.5v8M8 6.5h8");
/// A plus in the room below a row.
pub const MARK_PLUS_BOTTOM: Icon = Icon::new("M12 13.5v8M8 17.5h8");
/// A plus in the room left of a column.
pub const MARK_PLUS_LEFT: Icon = Icon::new("M2.5 12h8M6.5 8v8");
/// A plus in the room right of a column.
pub const MARK_PLUS_RIGHT: Icon = Icon::new("M13.5 12h8M17.5 8v8");
/// A cross in the room after a row.
pub const MARK_CROSS_RIGHT: Icon = Icon::new("M16.5 9l6 6M22.5 9l-6 6");
/// A cross in the room under a column.
pub const MARK_CROSS_BOTTOM: Icon = Icon::new("M9 16.5l6 6M15 16.5l-6 6");
