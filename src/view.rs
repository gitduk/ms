use std::io::IsTerminal;
use std::path::Path;

use anyhow::Result;

use crate::config::{self, Config, Pane, Window};
use crate::restore;

pub fn print(cfg: &Config) -> Result<()> {
    let width = restore::terminal_size().map_or(80, |(cols, _)| usize::from(cols).max(20));
    let home = config::home()?;
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let paint = |code: &str, text: &str| paint(color, code, text);
    for (i, w) in cfg.windows.iter().enumerate() {
        if i > 0 {
            println!();
        }
        let labels = labels(cfg, w, &home)?;
        let head = format!(
            "{} {}",
            paint("1;36", &w.name),
            paint("2", &format!("· {}", count(labels.len(), "pane")))
        );
        let cells = match w.layout.as_deref() {
            None if labels.len() == 1 => Ok(vec![Cell {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            }]),
            None => Err("no layout".to_owned()),
            Some(l) => match parse_layout(l) {
                Some(c) if c.len() == labels.len() => Ok(c),
                Some(c) => Err(format!("layout has {}", count(c.len(), "pane"))),
                None => Err(format!("layout: {l}")),
            },
        };
        match cells {
            Ok(cells) => {
                println!("{head}");
                for line in draw(&cells, &labels, width, color) {
                    println!("{line}");
                }
            }
            Err(why) => {
                println!("{head}  ({why})");
                for (n, (cmd, root)) in labels.iter().enumerate() {
                    let root = paint(ROOT, root);
                    match cmd {
                        Some(cmd) => println!("  {}. {root}  {}", n + 1, paint(CMD, cmd)),
                        None => println!("  {}. {root}", n + 1),
                    }
                }
            }
        }
    }
    Ok(())
}

const ROOT: &str = "34";
const CMD: &str = "32";

fn paint(color: bool, code: &str, text: &str) -> String {
    if color && !code.is_empty() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

pub fn count(n: usize, noun: &str) -> String {
    let s = if n == 1 { "" } else { "s" };
    format!("{n} {noun}{s}")
}

/// `(cmd, root)` per pane, the root fully resolved; no cmd for a bare shell.
fn labels(cfg: &Config, w: &Window, home: &Path) -> Result<Vec<Label>> {
    let label = |p: Option<&Pane>| -> Result<Label> {
        let cmd = p
            .and_then(|p| p.cmd.as_deref())
            .filter(|c| !c.is_empty())
            .map(str::to_owned);
        let root = config::tildify(&cfg.resolve_root(Some(w), p)?, home);
        Ok((cmd, root.display().to_string()))
    };
    if w.panes.is_empty() {
        return Ok(vec![label(None)?]);
    }
    w.panes.iter().map(|p| label(Some(p))).collect()
}

type Label = (Option<String>, String);

#[derive(Debug, PartialEq)]
struct Cell {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

/// The panes of a `#{window_layout}` string, in pane order; `None` for a
/// preset name or anything malformed.
fn parse_layout(layout: &str) -> Option<Vec<Cell>> {
    let (_checksum, body) = layout.split_once(',')?;
    let mut p = LayoutParser {
        s: body.as_bytes(),
        i: 0,
        leaves: Vec::new(),
    };
    p.cell()?;
    (p.i == p.s.len()).then_some(p.leaves)
}

struct LayoutParser<'a> {
    s: &'a [u8],
    i: usize,
    leaves: Vec<Cell>,
}

impl LayoutParser<'_> {
    fn eat(&mut self, c: u8) -> bool {
        let hit = self.s.get(self.i) == Some(&c);
        self.i += usize::from(hit);
        hit
    }

    fn num(&mut self) -> Option<usize> {
        let start = self.i;
        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()?
            .parse()
            .ok()
    }

    /// `WxH,X,Y` then `,id` for a pane, or `{...}` / `[...]` for a split.
    fn cell(&mut self) -> Option<()> {
        let w = self.num()?;
        self.eat(b'x').then_some(())?;
        let h = self.num()?;
        self.eat(b',').then_some(())?;
        let x = self.num()?;
        self.eat(b',').then_some(())?;
        let y = self.num()?;
        let close = match self.s.get(self.i) {
            Some(b'{') => b'}',
            Some(b'[') => b']',
            Some(b',') => {
                self.i += 1;
                self.num()?;
                self.leaves.push(Cell { x, y, w, h });
                return Some(());
            }
            _ => return None,
        };
        self.i += 1;
        loop {
            self.cell()?;
            if self.eat(close) {
                return Some(());
            }
            self.eat(b',').then_some(())?;
        }
    }
}

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

/// Scales the layout to `width` columns, giving every pane room for at
/// least one line of text.
fn draw(cells: &[Cell], labels: &[Label], width: usize, color: bool) -> Vec<String> {
    // Edges in tmux columns: a pane's border sits one past its last cell.
    let xs = bounds(cells.iter().flat_map(|c| [c.x, c.x + c.w + 1]));
    let ys = bounds(cells.iter().flat_map(|c| [c.y, c.y + c.h + 1]));
    let (tw, th) = (xs[xs.len() - 1] as f64, ys[ys.len() - 1] as f64);
    let sx = (width - 1) as f64 / tw;
    // About four rows per stacked pane, never taller than true scale.
    let sy = sx.min((4 * (ys.len() - 1)) as f64 / th);
    let (px, py) = (place(&xs, sx, 3), place(&ys, sy, 2));
    let at = |bs: &[usize], ps: &[usize], v: usize| ps[bs.partition_point(|&b| b < v)];

    let (cols, rows) = (px[px.len() - 1] + 1, py[py.len() - 1] + 1);
    let mut links = vec![vec![0u8; cols]; rows];
    let mut boxes = Vec::with_capacity(cells.len());
    for c in cells {
        let (x0, x1) = (at(&xs, &px, c.x), at(&xs, &px, c.x + c.w + 1));
        let (y0, y1) = (at(&ys, &py, c.y), at(&ys, &py, c.y + c.h + 1));
        for y in [y0, y1] {
            for (x, l) in links[y].iter_mut().enumerate().take(x1 + 1).skip(x0) {
                *l |= if x > x0 { LEFT } else { 0 } | if x < x1 { RIGHT } else { 0 };
            }
        }
        for x in [x0, x1] {
            for (y, row) in links.iter_mut().enumerate().take(y1 + 1).skip(y0) {
                row[x] |= if y > y0 { UP } else { 0 } | if y < y1 { DOWN } else { 0 };
            }
        }
        boxes.push((x0, x1, y0, y1));
    }

    let mut grid: Vec<Vec<(char, &str)>> = links
        .iter()
        .map(|row| row.iter().map(|&l| (junction(l), "")).collect())
        .collect();
    for (&(x0, x1, y0, y1), label) in boxes.iter().zip(labels) {
        let inner = x1 - x0 - 1;
        let pad = usize::from(inner >= 4);
        let (cmd, root) = label;
        // A one-row pane keeps the cmd: it tells panes apart, the root rarely does.
        let lines = match cmd {
            Some(cmd) if y1 - y0 > 2 => vec![(root, ROOT), (cmd, CMD)],
            Some(cmd) => vec![(cmd, CMD)],
            None => vec![(root, ROOT)],
        };
        for (row, (text, code)) in (y0 + 1..y1).zip(lines) {
            let text = fit(text, inner - 2 * pad);
            for (x, ch) in (x0 + 1 + pad..).zip(text.chars()) {
                grid[row][x] = (ch, code);
            }
        }
    }
    grid.iter()
        .map(|row| {
            row.chunk_by(|a, b| a.1 == b.1)
                .map(|run| {
                    paint(
                        color,
                        run[0].1,
                        &run.iter().map(|c| c.0).collect::<String>(),
                    )
                })
                .collect()
        })
        .collect()
}

fn bounds(edges: impl Iterator<Item = usize>) -> Vec<usize> {
    let mut v: Vec<usize> = edges.collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Scales each edge, pushing it right of its predecessor by `min_gap`.
fn place(edges: &[usize], scale: f64, min_gap: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(edges.len());
    for &e in edges {
        let p = (e as f64 * scale).round() as usize;
        out.push(out.last().map_or(p, |&last| p.max(last + min_gap)));
    }
    out
}

fn junction(links: u8) -> char {
    match links {
        0 => ' ',
        l if l == DOWN | RIGHT => '┌',
        l if l == DOWN | LEFT => '┐',
        l if l == UP | RIGHT => '└',
        l if l == UP | LEFT => '┘',
        l if l == UP | DOWN | RIGHT => '├',
        l if l == UP | DOWN | LEFT => '┤',
        l if l == DOWN | LEFT | RIGHT => '┬',
        l if l == UP | LEFT | RIGHT => '┴',
        l if l == UP | DOWN | LEFT | RIGHT => '┼',
        l if l & (UP | DOWN) != 0 => '│',
        _ => '─',
    }
}

fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut s: String = text.chars().take(width.saturating_sub(1)).collect();
    if width > 0 {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_layout_in_pane_order() {
        let cells = parse_layout("f655,234x53,0,0[234x26,0,0{117x26,0,0,8,116x26,118,0,26},234x26,0,27{117x26,0,27,11,116x26,118,27,24}]").unwrap();
        assert_eq!(
            cells,
            [
                Cell {
                    x: 0,
                    y: 0,
                    w: 117,
                    h: 26
                },
                Cell {
                    x: 118,
                    y: 0,
                    w: 116,
                    h: 26
                },
                Cell {
                    x: 0,
                    y: 27,
                    w: 117,
                    h: 26
                },
                Cell {
                    x: 118,
                    y: 27,
                    w: 116,
                    h: 26
                },
            ]
        );
    }

    #[test]
    fn rejects_presets_and_garbage() {
        assert_eq!(parse_layout("tiled"), None);
        assert_eq!(parse_layout("main-vertical"), None);
        assert_eq!(parse_layout("abcd,80x24,0,0{40x24,0,0,1"), None);
        assert_eq!(parse_layout("abcd,80x24,0,0,1junk"), None);
    }

    #[test]
    fn draws_shared_borders_as_junctions() {
        let cells = parse_layout("8213,80x24,0,0{40x24,0,0,3,39x24,41,0,5}").unwrap();
        let labels = [
            (Some("sleep 998".to_owned()), "/tmp".to_owned()),
            (None, "~".to_owned()),
        ];
        assert_eq!(
            draw(&cells, &labels, 30, false),
            [
                "┌──────────────┬─────────────┐",
                "│ /tmp         │ ~           │",
                "│ sleep 998    │             │",
                "│              │             │",
                "└──────────────┴─────────────┘",
            ]
        );
    }
}
