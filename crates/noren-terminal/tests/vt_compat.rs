use noren_terminal::{TerminalEngine, TerminalSnapshot, TerminalState};

struct Harness {
    engine: Box<dyn TerminalEngine>,
}

impl Harness {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            engine: Box::new(TerminalState::new(rows, cols).expect("valid compatibility grid")),
        }
    }

    fn feed(&mut self, bytes: &[u8], context: &str) {
        self.engine.feed_bytes(bytes);
        self.assert_public_invariants(context);
    }

    fn feed_bytewise(&mut self, bytes: &[u8], context: &str) {
        for (index, byte) in bytes.iter().enumerate() {
            self.feed(
                std::slice::from_ref(byte),
                &format!("{context}, byte {index}"),
            );
        }
    }

    fn resize(&mut self, rows: u16, cols: u16, context: &str) {
        self.engine
            .resize(rows, cols)
            .expect("valid compatibility resize");
        self.assert_public_invariants(context);
    }

    fn snapshot(&self) -> TerminalSnapshot {
        self.engine.snapshot()
    }

    fn assert_public_invariants(&self, context: &str) {
        let snapshot = self.snapshot();
        let (rows, cols) = self.engine.size();
        let screen = snapshot.screen();

        assert_eq!(
            (snapshot.rows(), snapshot.cols()),
            (rows, cols),
            "{context}"
        );
        assert_eq!((screen.rows(), screen.cols()), (rows, cols), "{context}");
        assert_eq!(
            screen.cells().len(),
            usize::from(rows) * usize::from(cols),
            "{context}"
        );

        let cursor = snapshot.cursor();
        assert!(cursor.row() < rows, "{context}: cursor row");
        assert!(cursor.column() < cols, "{context}: cursor column");

        let region = snapshot.scroll_region();
        assert!(region.top() <= region.bottom(), "{context}: scroll order");
        assert!(region.bottom() < rows, "{context}: scroll bounds");
        assert_eq!(
            region.height(),
            region.bottom() - region.top() + 1,
            "{context}: scroll height"
        );

        for row in 0..rows {
            for column in 0..cols {
                let index = usize::from(row) * usize::from(cols) + usize::from(column);
                assert_eq!(
                    screen.cell(row, column),
                    screen.cells().get(index),
                    "{context}: row-major cell ({row}, {column})"
                );
            }
        }
        assert!(screen.cell(rows, 0).is_none(), "{context}: row bound");
        assert!(screen.cell(0, cols).is_none(), "{context}: column bound");
    }
}

#[derive(Clone, Copy)]
struct ExpectedFrame {
    lines: &'static [&'static str],
    cursor: (u16, u16),
    region: (u16, u16),
    wrap_pending: bool,
    alternate_screen: bool,
}

impl ExpectedFrame {
    fn assert_matches(self, snapshot: &TerminalSnapshot, context: &str) {
        let lines: Vec<_> = snapshot.lines().iter().map(String::as_str).collect();
        let cursor = snapshot.cursor();
        let region = snapshot.scroll_region();

        assert_eq!(lines.as_slice(), self.lines, "{context}: visible lines");
        assert_eq!(
            (cursor.row(), cursor.column()),
            self.cursor,
            "{context}: cursor"
        );
        assert_eq!(
            (region.top(), region.bottom()),
            self.region,
            "{context}: scroll region"
        );
        assert_eq!(
            snapshot.is_wrap_pending(),
            self.wrap_pending,
            "{context}: delayed wrap"
        );
        assert_eq!(
            snapshot.modes().is_alternate_screen_active(),
            self.alternate_screen,
            "{context}: alternate screen mode"
        );
    }
}

struct SplitCase {
    name: &'static str,
    rows: u16,
    cols: u16,
    prefix: &'static [u8],
    sequence: &'static [u8],
    suffix: &'static [u8],
    expected: ExpectedFrame,
}

const SPLIT_CASES: &[SplitCase] = &[
    SplitCase {
        name: "absolute cursor position",
        rows: 3,
        cols: 5,
        prefix: b"AB",
        sequence: b"\x1b[2;4H",
        suffix: b"Z",
        expected: ExpectedFrame {
            lines: &["AB", "   Z"],
            cursor: (1, 4),
            region: (0, 2),
            wrap_pending: false,
            alternate_screen: false,
        },
    },
    SplitCase {
        name: "scroll margins",
        rows: 5,
        cols: 3,
        prefix: b"AAA\x1b[2;1HBBB\x1b[3;1HCCC\x1b[4;1HDDD\x1b[5;1HEEE",
        sequence: b"\x1b[2;4r",
        suffix: b"\x1b[4;1H\x1bD",
        expected: ExpectedFrame {
            lines: &["AAA", "CCC", "DDD", "", "EEE"],
            cursor: (3, 0),
            region: (1, 3),
            wrap_pending: false,
            alternate_screen: false,
        },
    },
    SplitCase {
        name: "alternate screen entry",
        rows: 4,
        cols: 8,
        prefix: b"PRIMARY\x1b[3;4H",
        sequence: b"\x1b[?1049h",
        suffix: b"ALT",
        expected: ExpectedFrame {
            lines: &["ALT"],
            cursor: (0, 3),
            region: (0, 3),
            wrap_pending: false,
            alternate_screen: true,
        },
    },
    SplitCase {
        name: "alternate screen exit",
        rows: 4,
        cols: 8,
        prefix: b"PRIMARY\x1b[3;4H\x1b[?1049hALT\x1b[2;2H!",
        sequence: b"\x1b[?1049l",
        suffix: b"X",
        expected: ExpectedFrame {
            lines: &["PRIMARY", "", "   X"],
            cursor: (2, 4),
            region: (0, 3),
            wrap_pending: false,
            alternate_screen: false,
        },
    },
    SplitCase {
        name: "saved cursor restore",
        rows: 3,
        cols: 5,
        prefix: b"\x1b[2;3H\x1b[s\x1b[3;5H",
        sequence: b"\x1b[u",
        suffix: b"Z",
        expected: ExpectedFrame {
            lines: &["", "  Z"],
            cursor: (1, 3),
            region: (0, 2),
            wrap_pending: false,
            alternate_screen: false,
        },
    },
];

#[test]
fn escape_sequences_accept_every_possible_feed_partition() {
    for case in SPLIT_CASES {
        let boundary_count = case.sequence.len() - 1;
        let partition_count = 1_usize << boundary_count;

        for partition in 0..partition_count {
            let context = format!("{}, partition {partition:#b}", case.name);
            let mut harness = Harness::new(case.rows, case.cols);
            harness.feed(case.prefix, &context);

            let mut chunk_start = 0;
            for boundary in 1..case.sequence.len() {
                if partition & (1 << (boundary - 1)) != 0 {
                    harness.feed(&case.sequence[chunk_start..boundary], &context);
                    chunk_start = boundary;
                }
            }
            harness.feed(&case.sequence[chunk_start..], &context);
            harness.feed(case.suffix, &context);

            case.expected.assert_matches(&harness.snapshot(), &context);
        }
    }
}

struct InvariantCase {
    name: &'static str,
    rows: u16,
    cols: u16,
    bytes: &'static [u8],
}

const INVARIANT_CASES: &[InvariantCase] = &[
    InvariantCase {
        name: "cursor clamps and delayed wrap",
        rows: 3,
        cols: 4,
        bytes: b"abcdE\x1b[999;999H!\x1b[999A\x1b[999D?",
    },
    InvariantCase {
        name: "regional index and reverse index",
        rows: 5,
        cols: 3,
        bytes: b"AAA\x1b[2;1HBBB\x1b[3;1HCCC\x1b[4;1HDDD\x1b[5;1HEEE\x1b[2;4r\x1b[4;1H\x1bD\x1b[2;1H\x1bM",
    },
    InvariantCase {
        name: "alternate screen round trip",
        rows: 4,
        cols: 6,
        bytes: b"BASE\x1b[2;5H\x1b[?1049hALT\x1b[3;3H\x1b7\x1b[4;6H!\x1b8\x1b[?1049l",
    },
];

#[test]
fn cursor_and_buffer_stay_bounded_after_every_input_byte() {
    for case in INVARIANT_CASES {
        let mut harness = Harness::new(case.rows, case.cols);
        harness.feed_bytewise(case.bytes, case.name);
    }
}

#[test]
fn resize_after_regional_scroll_preserves_overlap_and_resets_margins() {
    let mut harness = Harness::new(5, 4);
    harness.feed_bytewise(
        b"AAAA\x1b[2;1HBBBB\x1b[3;1HCCCC\x1b[4;1HDDDD\x1b[5;1HEEEE\x1b[2;4r\x1b[4;1H\x1bD",
        "regional scroll before resize",
    );

    let before_resize = harness.snapshot();
    assert_eq!(
        full_grid(&before_resize),
        ["AAAA", "CCCC", "DDDD", "    ", "EEEE"]
    );
    assert_cursor_and_region(&before_resize, (3, 0), (1, 3));

    harness.resize(4, 3, "shrink after regional scroll");
    let resized = harness.snapshot();
    assert_eq!(full_grid(&resized), ["AAA", "CCC", "DDD", "   "]);
    assert_cursor_and_region(&resized, (3, 0), (0, 3));
    assert!(!resized.is_wrap_pending());

    harness.feed(b"\n", "full-screen scroll after resize");
    let after_scroll = harness.snapshot();
    assert_eq!(full_grid(&after_scroll), ["CCC", "DDD", "   ", "   "]);
    assert_cursor_and_region(&after_scroll, (3, 0), (0, 3));
}

#[test]
fn alternate_screen_isolates_cells_and_cursor_save_state() {
    let mut harness = Harness::new(4, 6);
    harness.feed_bytewise(b"BASE\x1b[2;5H", "prepare primary screen");
    let primary = harness.snapshot();

    harness.feed_bytewise(b"\x1b[?1049h", "enter alternate screen");
    let blank_alternate = harness.snapshot();
    assert!(blank_alternate.modes().is_alternate_screen_active());
    assert!(
        blank_alternate
            .screen()
            .cells()
            .iter()
            .all(|cell| cell.is_blank())
    );
    assert_cursor_and_region(&blank_alternate, (0, 0), (0, 3));

    harness.feed_bytewise(
        b"ALT\x1b[3;3H\x1b7\x1b[4;6H!\x1b8",
        "write and restore cursor on alternate screen",
    );
    let alternate = harness.snapshot();
    assert_eq!(
        full_grid(&alternate),
        ["ALT   ", "      ", "      ", "     !"]
    );
    assert_cursor_and_region(&alternate, (2, 2), (0, 3));
    assert!(!alternate.is_wrap_pending());

    harness.feed_bytewise(b"\x1b[?1049l", "leave alternate screen");
    assert_eq!(harness.snapshot(), primary);

    harness.feed_bytewise(
        b"\x1b[4;1H\x1b8P",
        "restore primary entry cursor after alternate save",
    );
    let restored_primary = harness.snapshot();
    assert_eq!(
        full_grid(&restored_primary),
        ["BASE  ", "    P ", "      ", "      "]
    );
    assert_cursor_and_region(&restored_primary, (1, 5), (0, 3));
    assert!(!restored_primary.modes().is_alternate_screen_active());
}

fn full_grid(snapshot: &TerminalSnapshot) -> Vec<String> {
    (0..snapshot.rows())
        .map(|row| {
            (0..snapshot.cols())
                .map(|column| {
                    snapshot
                        .screen()
                        .cell(row, column)
                        .expect("public grid cell is in bounds")
                        .text()
                })
                .collect()
        })
        .collect()
}

fn assert_cursor_and_region(
    snapshot: &TerminalSnapshot,
    expected_cursor: (u16, u16),
    expected_region: (u16, u16),
) {
    let cursor = snapshot.cursor();
    let region = snapshot.scroll_region();
    assert_eq!((cursor.row(), cursor.column()), expected_cursor);
    assert_eq!((region.top(), region.bottom()), expected_region);
}
