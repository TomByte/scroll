//! The published palettes the named themes are built from: for each, the handful of its own
//! colours `derivation` turns into a whole theme. Every value is copied from the source cited
//! above it, never adjusted here; where a colour needs nudging to stay readable, `derivation`
//! does it to the derived token.

/// One theme's source colours, as `0xRRGGBB`.
pub(super) struct SourceColors {
    /// Main content area.
    pub(super) surface: u32,
    /// Sidebar and other secondary panes.
    pub(super) surface_secondary: u32,
    /// Popovers, menus, modals.
    pub(super) surface_elevated: u32,
    pub(super) text: u32,
    pub(super) text_muted: u32,
    /// Placeholders.
    pub(super) text_subtle: u32,
    pub(super) accent: u32,
    pub(super) link: u32,
    pub(super) danger: u32,
    pub(super) syntax: SourceSyntax,
    /// Four hues from the palette that stand apart from each other and from the accent.
    pub(super) collaborators: [u32; 4],
}

pub(super) struct SourceSyntax {
    pub(super) keyword: u32,
    pub(super) string: u32,
    pub(super) comment: u32,
    pub(super) constant: u32,
    pub(super) function: u32,
    pub(super) type_name: u32,
}

// Ayu: ayu-colors' themes/*.yaml compute most colours from a base palette, so the values are
// the computed ones its npm package publishes, https://unpkg.com/ayu@9.1.0/dist/generated/
// (light.js, mirage.js, dark.js), from https://github.com/ayu-theme/ayu-colors. Roles: editor
// bg/fg for surface and text, ui bg/fg for sidebar and muted text, the accent tint, `entity`
// (types) for links.

pub(super) const AYU_LIGHT: SourceColors = SourceColors {
    surface: 0xfcfcfc,
    surface_secondary: 0xf8f9fa,
    surface_elevated: 0xffffff,
    text: 0x5c6166,
    text_muted: 0x828e9f,
    text_subtle: 0xadaeb1,
    accent: 0xf29718,
    link: 0x22a4e6,
    danger: 0xe65050,
    syntax: SourceSyntax {
        keyword: 0xfa8532,
        string: 0x86b300,
        comment: 0xadaeb1,
        constant: 0xa37acc,
        function: 0xeba400,
        type_name: 0x22a4e6,
    },
    collaborators: [0xfa8532, 0x86b300, 0xa37acc, 0xf07171],
};

pub(super) const AYU_MIRAGE: SourceColors = SourceColors {
    surface: 0x242936,
    surface_secondary: 0x1f2430,
    surface_elevated: 0x282e3b,
    text: 0xcccac2,
    text_muted: 0x707a8c,
    text_subtle: 0x6e7c8f,
    accent: 0xffcc66,
    link: 0x73d0ff,
    danger: 0xff6666,
    syntax: SourceSyntax {
        keyword: 0xffa659,
        string: 0xd5ff80,
        comment: 0x6e7c8f,
        constant: 0xdfbfff,
        function: 0xffcd66,
        type_name: 0x73d0ff,
    },
    collaborators: [0xffa659, 0xd5ff80, 0xdfbfff, 0xf28779],
};

pub(super) const AYU_DARK: SourceColors = SourceColors {
    surface: 0x10141c,
    surface_secondary: 0x0d1017,
    surface_elevated: 0x141821,
    text: 0xbfbdb6,
    text_muted: 0x5a6378,
    text_subtle: 0x5a6673,
    accent: 0xe6b450,
    link: 0x59c2ff,
    danger: 0xd95757,
    syntax: SourceSyntax {
        keyword: 0xff8f40,
        string: 0xaad94c,
        comment: 0x5a6673,
        constant: 0xd2a6ff,
        function: 0xffb454,
        type_name: 0x59c2ff,
    },
    collaborators: [0xff8f40, 0xaad94c, 0xd2a6ff, 0xf07178],
};

// Catppuccin: https://github.com/catppuccin/palette/blob/main/palette.json (v1.8.0). Roles
// follow https://github.com/catppuccin/catppuccin/blob/main/docs/style-guide.md: base, mantle
// for secondary panes, subtext0 for labels, overlay1 for subtle text, blue links, and its code
// colours. Mauve is the accent, as in most Catppuccin ports; dark flavours raise popovers onto
// surface0, and Latte has nothing lighter than base.

pub(super) const CATPPUCCIN_LATTE: SourceColors = SourceColors {
    surface: 0xeff1f5,
    surface_secondary: 0xe6e9ef,
    surface_elevated: 0xeff1f5,
    text: 0x4c4f69,
    text_muted: 0x6c6f85,
    text_subtle: 0x8c8fa1,
    accent: 0x8839ef,
    link: 0x1e66f5,
    danger: 0xd20f39,
    syntax: SourceSyntax {
        keyword: 0x8839ef,
        string: 0x40a02b,
        comment: 0x7c7f93,
        constant: 0xfe640b,
        function: 0x1e66f5,
        type_name: 0xdf8e1d,
    },
    collaborators: [0xfe640b, 0x40a02b, 0xea76cb, 0x209fb5],
};

pub(super) const CATPPUCCIN_FRAPPE: SourceColors = SourceColors {
    surface: 0x303446,
    surface_secondary: 0x292c3c,
    surface_elevated: 0x414559,
    text: 0xc6d0f5,
    text_muted: 0xa5adce,
    text_subtle: 0x838ba7,
    accent: 0xca9ee6,
    link: 0x8caaee,
    danger: 0xe78284,
    syntax: SourceSyntax {
        keyword: 0xca9ee6,
        string: 0xa6d189,
        comment: 0x949cbb,
        constant: 0xef9f76,
        function: 0x8caaee,
        type_name: 0xe5c890,
    },
    collaborators: [0xef9f76, 0xa6d189, 0xf4b8e4, 0x85c1dc],
};

pub(super) const CATPPUCCIN_MACCHIATO: SourceColors = SourceColors {
    surface: 0x24273a,
    surface_secondary: 0x1e2030,
    surface_elevated: 0x363a4f,
    text: 0xcad3f5,
    text_muted: 0xa5adcb,
    text_subtle: 0x8087a2,
    accent: 0xc6a0f6,
    link: 0x8aadf4,
    danger: 0xed8796,
    syntax: SourceSyntax {
        keyword: 0xc6a0f6,
        string: 0xa6da95,
        comment: 0x939ab7,
        constant: 0xf5a97f,
        function: 0x8aadf4,
        type_name: 0xeed49f,
    },
    collaborators: [0xf5a97f, 0xa6da95, 0xf5bde6, 0x7dc4e4],
};

pub(super) const CATPPUCCIN_MOCHA: SourceColors = SourceColors {
    surface: 0x1e1e2e,
    surface_secondary: 0x181825,
    surface_elevated: 0x313244,
    text: 0xcdd6f4,
    text_muted: 0xa6adc8,
    text_subtle: 0x7f849c,
    accent: 0xcba6f7,
    link: 0x89b4fa,
    danger: 0xf38ba8,
    syntax: SourceSyntax {
        keyword: 0xcba6f7,
        string: 0xa6e3a1,
        comment: 0x9399b2,
        constant: 0xfab387,
        function: 0x89b4fa,
        type_name: 0xf9e2af,
    },
    collaborators: [0xfab387, 0xa6e3a1, 0xf5c2e7, 0x74c7ec],
};

// Dracula: https://spec.draculatheme.com/ (1.2 Color Palette and its syntax roles). The spec
// has no sidebar shade or muted grey: the sidebar takes AnsiBlack, muted text the comment
// colour (which `derivation` lightens to stay readable), links the MarkupLinkUrl cyan.

pub(super) const DRACULA: SourceColors = SourceColors {
    surface: 0x282a36,
    surface_secondary: 0x21222c,
    surface_elevated: 0x282a36,
    text: 0xf8f8f2,
    text_muted: 0x6272a4,
    text_subtle: 0x6272a4,
    accent: 0xbd93f9,
    link: 0x8be9fd,
    danger: 0xff5555,
    syntax: SourceSyntax {
        keyword: 0xff79c6,
        string: 0xf1fa8c,
        comment: 0x6272a4,
        constant: 0xbd93f9,
        function: 0x50fa7b,
        type_name: 0x8be9fd,
    },
    collaborators: [0xffb86c, 0x50fa7b, 0xff79c6, 0x8be9fd],
};

// Gruvbox: https://github.com/morhetz/gruvbox/blob/master/colors/gruvbox.vim. Dark uses the
// bright accents, light the faded ones, as gruvbox.vim does; code colours follow its links
// (Keyword red, String and Function green, Constant purple, Type yellow, Comment gray).

pub(super) const GRUVBOX_DARK: SourceColors = SourceColors {
    surface: 0x282828,
    surface_secondary: 0x1d2021,
    surface_elevated: 0x3c3836,
    text: 0xebdbb2,
    text_muted: 0xa89984,
    text_subtle: 0x928374,
    accent: 0xfe8019,
    link: 0x83a598,
    danger: 0xfb4934,
    syntax: SourceSyntax {
        keyword: 0xfb4934,
        string: 0xb8bb26,
        comment: 0x928374,
        constant: 0xd3869b,
        function: 0xb8bb26,
        type_name: 0xfabd2f,
    },
    collaborators: [0xfe8019, 0x8ec07c, 0xd3869b, 0x83a598],
};

pub(super) const GRUVBOX_LIGHT: SourceColors = SourceColors {
    surface: 0xfbf1c7,
    surface_secondary: 0xebdbb2,
    surface_elevated: 0xf9f5d7,
    text: 0x3c3836,
    text_muted: 0x7c6f64,
    text_subtle: 0x928374,
    accent: 0xaf3a03,
    link: 0x076678,
    danger: 0x9d0006,
    syntax: SourceSyntax {
        keyword: 0x9d0006,
        string: 0x79740e,
        comment: 0x928374,
        constant: 0x8f3f71,
        function: 0x79740e,
        type_name: 0xb57614,
    },
    collaborators: [0xd65d0e, 0x689d6a, 0xb16286, 0x458588],
};

// Kanagawa: https://github.com/rebelot/kanagawa.nvim/blob/master/lua/kanagawa/colors.lua for
// the values, and lua/kanagawa/themes.lua beside it for each variant's roles (ui.bg, bg_dim,
// bg_p1, fg, fg_dim, syn.*, diag.error). Popovers take bg_p1, lighter than bg, so they lift off
// the page as the other dark themes' do.

pub(super) const KANAGAWA_WAVE: SourceColors = SourceColors {
    surface: 0x1f1f28,
    surface_secondary: 0x181820,
    surface_elevated: 0x2a2a37,
    text: 0xdcd7ba,
    text_muted: 0xc8c093,
    text_subtle: 0x727169,
    accent: 0x7e9cd8,
    link: 0x7fb4ca,
    danger: 0xe82424,
    syntax: SourceSyntax {
        keyword: 0x957fb8,
        string: 0x98bb6c,
        comment: 0x727169,
        constant: 0xffa066,
        function: 0x7e9cd8,
        type_name: 0x7aa89f,
    },
    collaborators: [0xffa066, 0x98bb6c, 0x957fb8, 0xd27e99],
};

/// Dragon's fg_dim (oldWhite) is brighter than its fg, so muted text takes dragonGray instead.
pub(super) const KANAGAWA_DRAGON: SourceColors = SourceColors {
    surface: 0x181616,
    surface_secondary: 0x12120f,
    surface_elevated: 0x282727,
    text: 0xc5c9c5,
    text_muted: 0xa6a69c,
    text_subtle: 0x737c73,
    accent: 0x8ba4b0,
    link: 0x8ba4b0,
    danger: 0xe82424,
    syntax: SourceSyntax {
        keyword: 0x8992a7,
        string: 0x8a9a7b,
        comment: 0x737c73,
        constant: 0xb6927b,
        function: 0x8ba4b0,
        type_name: 0x8ea4a2,
    },
    collaborators: [0xc4746e, 0x87a987, 0x8992a7, 0xc4b28a],
};

/// Lotus's fg_dim is darker than its fg, so muted text takes lotusGray2 (its float border).
pub(super) const KANAGAWA_LOTUS: SourceColors = SourceColors {
    surface: 0xf2ecbc,
    surface_secondary: 0xdcd5ac,
    surface_elevated: 0xf2ecbc,
    text: 0x545464,
    text_muted: 0x716e61,
    text_subtle: 0x8a8980,
    accent: 0x4d699b,
    link: 0x4d699b,
    danger: 0xe82424,
    syntax: SourceSyntax {
        keyword: 0x624c83,
        string: 0x6f894e,
        comment: 0x8a8980,
        constant: 0xcc6d00,
        function: 0x4d699b,
        type_name: 0x597b75,
    },
    collaborators: [0xcc6d00, 0x6f894e, 0x624c83, 0xb35b79],
};

// Nord: https://github.com/nordtheme/nord/blob/develop/src/nord.css for nord0–nord15, and
// https://github.com/nordtheme/vim/blob/main/colors/nord.vim for its syntax roles and the
// brightened comment grey (#616E88), which Nord's own palette lacks. Nord has no grey between
// that and its text, so `derivation` lightens it for muted text.

pub(super) const NORD: SourceColors = SourceColors {
    surface: 0x2e3440,
    surface_secondary: 0x3b4252,
    surface_elevated: 0x3b4252,
    text: 0xd8dee9,
    text_muted: 0x616e88,
    text_subtle: 0x616e88,
    accent: 0x88c0d0,
    link: 0x88c0d0,
    danger: 0xbf616a,
    syntax: SourceSyntax {
        keyword: 0x81a1c1,
        string: 0xa3be8c,
        comment: 0x616e88,
        constant: 0xb48ead,
        function: 0x88c0d0,
        type_name: 0x8fbcbb,
    },
    collaborators: [0xd08770, 0xa3be8c, 0xb48ead, 0xebcb8b],
};

// Rosé Pine: https://github.com/rose-pine/palette/blob/main/palette.json (main and dawn),
// with code roles from https://github.com/rose-pine/neovim/blob/main/lua/rose-pine.lua.
// Rosé Pine's "subtle" is brighter than its "muted", so subtle is the muted text here. Its
// Neovim port draws constants in gold like strings; iris keeps them apart.

pub(super) const ROSE_PINE: SourceColors = SourceColors {
    surface: 0x191724,
    surface_secondary: 0x1f1d2e,
    surface_elevated: 0x26233a,
    text: 0xe0def4,
    text_muted: 0x908caa,
    text_subtle: 0x6e6a86,
    accent: 0xc4a7e7,
    link: 0x9ccfd8,
    danger: 0xeb6f92,
    syntax: SourceSyntax {
        keyword: 0x31748f,
        string: 0xf6c177,
        comment: 0x908caa,
        constant: 0xc4a7e7,
        function: 0xebbcba,
        type_name: 0x9ccfd8,
    },
    collaborators: [0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xeb6f92],
};

pub(super) const ROSE_PINE_DAWN: SourceColors = SourceColors {
    surface: 0xfaf4ed,
    surface_secondary: 0xf2e9e1,
    surface_elevated: 0xfffaf3,
    text: 0x464261,
    text_muted: 0x797593,
    text_subtle: 0x9893a5,
    accent: 0x907aa9,
    link: 0x286983,
    danger: 0xb4637a,
    syntax: SourceSyntax {
        keyword: 0x286983,
        string: 0xea9d34,
        comment: 0x797593,
        constant: 0x907aa9,
        function: 0xd7827e,
        type_name: 0x56949f,
    },
    collaborators: [0xea9d34, 0x56949f, 0x907aa9, 0xb4637a],
};

// Tokyo Night (the "night" style): https://github.com/folke/tokyonight.nvim/blob/main/lua/tokyonight/colors/storm.lua
// with night.lua's bg overrides beside it, and roles from lua/tokyonight/groups/base.lua
// (Statement magenta, String green, Function blue, Type blue1, Constant orange, error red1).
// Popovers take bg_highlight rather than its darker bg_popup, to lift off the page.

pub(super) const TOKYO_NIGHT: SourceColors = SourceColors {
    surface: 0x1a1b26,
    surface_secondary: 0x16161e,
    surface_elevated: 0x292e42,
    text: 0xc0caf5,
    text_muted: 0xa9b1d6,
    text_subtle: 0x737aa2,
    accent: 0x7aa2f7,
    link: 0x7aa2f7,
    danger: 0xdb4b4b,
    syntax: SourceSyntax {
        keyword: 0xbb9af7,
        string: 0x9ece6a,
        comment: 0x565f89,
        constant: 0xff9e64,
        function: 0x7aa2f7,
        type_name: 0x2ac3de,
    },
    collaborators: [0xff9e64, 0x9ece6a, 0xbb9af7, 0xf7768e],
};
