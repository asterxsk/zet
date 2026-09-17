//! Temporary adversarial probes. Deleted after investigation.

use zet_vt::{CellFlags, Parser, Term};

fn feed(t: &mut Term, p: &mut Parser, bytes: &[u8]) {
    p.advance_slice(bytes, t);
}

fn row_state(t: &Term, row: usize) -> String {
    let cols = t.grid().cols();
    let r = t.grid().row(row);
    let mut out = String::new();
    for c in 0..cols {
        let cell = r.get(c);
        let mark = if cell.flags.contains(CellFlags::WIDE_CHAR) {
            'L'
        } else if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
            'S'
        } else if cell.flags.contains(CellFlags::LEADING_WIDE_CHAR_SPACER) {
            'X'
        } else {
            '.'
        };
        out.push_str(&format!("{}{} ", cell.ch, mark));
    }
    out
}

fn bad_states(t: &Term, row: usize) -> Vec<String> {
    let cols = t.grid().cols();
    let r = t.grid().row(row);
    let mut bad = Vec::new();
    for c in 0..cols {
        let cell = r.get(c);
        let lead = cell.flags.contains(CellFlags::WIDE_CHAR);
        let spacer = cell.flags.contains(CellFlags::WIDE_CHAR_SPACER);
        if lead {
            if c + 1 >= cols {
                bad.push(format!("LEAD_AT_EDGE col={c}"));
            } else if !r.get(c + 1).is_wide_spacer() {
                bad.push(format!("LEAD_NO_SPACER col={c} next='{}'", r.get(c + 1).ch));
            }
        }
        if spacer && (c == 0 || !r.get(c - 1).flags.contains(CellFlags::WIDE_CHAR)) {
            bad.push(format!("ORPHAN_SPACER col={c}"));
        }
    }
    bad
}

#[test]
fn sweep() {
    let mut out = String::new();
    for cols in [4usize, 6, 8] {
        for text in ["\u{4e2d}a", "a\u{4e2d}", "\u{4e2d}\u{4e2d}", "a\u{4e2d}b"] {
            for start in 0..cols {
                for n in 1..3usize {
                    for op in ["@", "P"] {
                        let mut t = Term::new(cols, 2);
                        let mut p = Parser::new();
                        feed(&mut t, &mut p, text.as_bytes());
                        feed(
                            &mut t,
                            &mut p,
                            format!("\x1b[1;{}H\x1b[{n}{op}", start + 1).as_bytes(),
                        );
                        let bad = bad_states(&t, 0);
                        if !bad.is_empty() {
                            out.push_str(&format!(
                                "cols={cols} text={text:?} start={start} n={n} op={op}\n   \
                                 {}\n   {bad:?}\n",
                                row_state(&t, 0)
                            ));
                        }
                    }
                }
            }
        }
    }
    std::fs::write("D:/Apps/tmp/zet_sweep.log", &out).unwrap();
    eprintln!("wrote {} bytes", out.len());
    assert!(out.is_empty(), "bad states found, see log");
}
