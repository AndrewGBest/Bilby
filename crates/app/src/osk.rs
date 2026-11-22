//! The TV's on-screen keyboard: its keys and where the highlight goes. The same
//! keyboard as Longboard's (its `src/shared/osk.ts`), so the two apps type alike:
//! letters in alphabetical order, ten keys to a row, three faces.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Char,
    Space,
    Delete,
    Submit,
    Shift,
    Symbols,
    Letters,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Key {
    pub kind: Kind,
    /// What is shown on the key, and for a letter what it types.
    pub label: String,
    /// Width in columns; every row is [`COLUMNS`] wide.
    pub span: usize,
}

pub const COLUMNS: usize = 10;

/// Small letters, capitals, symbols.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Face {
    #[default]
    Lower,
    Upper,
    Symbols,
}

/// Up, down, left, right: as the interface numbers them, 0 to 3.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub fn from(number: i32) -> Option<Dir> {
        [Dir::Up, Dir::Down, Dir::Left, Dir::Right].get(usize::try_from(number).ok()?).copied()
    }
}

fn chars(text: &str) -> Vec<Key> {
    text.chars().map(|c| Key { kind: Kind::Char, label: c.to_string(), span: 1 }).collect()
}

fn wide(kind: Kind, label: &str, span: usize) -> Key {
    Key { kind, label: label.to_string(), span }
}

pub fn rows(face: Face) -> Vec<Vec<Key>> {
    let ends = |space: usize| {
        [wide(Kind::Space, "space", space), wide(Kind::Delete, "delete", 2), wide(Kind::Submit, "search", 2)]
    };
    if face == Face::Symbols {
        let mut last = vec![wide(Kind::Letters, "abc", 1)];
        last.extend(chars("`~"));
        last.extend(ends(3));
        return vec![chars("1234567890"), chars("!@#$%^&*()"), chars("-_=+[]{}\\|"), chars(";:'\",.<>/?"), last];
    }
    let shift = face == Face::Upper;
    let c = |text: &str| chars(&if shift { text.to_uppercase() } else { text.to_string() });
    let mut third = c("uvwxyz");
    third.extend(chars("@.-_"));
    let mut last = vec![wide(Kind::Shift, if shift { "SHIFT" } else { "shift" }, 1), wide(Kind::Symbols, "!#?", 1)];
    last.extend(ends(4));
    vec![c("abcdefghij"), c("klmnopqrst"), third, chars("1234567890"), last]
}

/// The left edge of each key in a row, in columns.
fn starts(row: &[Key]) -> Vec<usize> {
    let mut x = 0;
    row.iter()
        .map(|key| {
            let start = x;
            x += key.span;
            start
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Model {
    pub face: Face,
    pub row: usize,
    pub index: usize,
}

impl Model {
    pub fn rows(&self) -> Vec<Vec<Key>> {
        rows(self.face)
    }

    pub fn key(&self) -> Key {
        self.rows()[self.row][self.index].clone()
    }

    /// Where the highlighted key is, counting every key from the first.
    pub fn flat(&self) -> usize {
        self.rows()[..self.row].iter().map(Vec::len).sum::<usize>() + self.index
    }

    /// Put the highlight on the key that is `flat` keys from the first.
    pub fn point_at(&mut self, mut flat: usize) {
        for (row, keys) in self.rows().iter().enumerate() {
            if flat < keys.len() {
                (self.row, self.index) = (row, flat);
                return;
            }
            flat -= keys.len();
        }
    }

    /// Put the highlight on the first key of this kind on the face that is showing.
    pub fn point_to(&mut self, kind: Kind) {
        for (row, keys) in self.rows().iter().enumerate() {
            if let Some(index) = keys.iter().position(|key| key.kind == kind) {
                (self.row, self.index) = (row, index);
                return;
            }
        }
    }

    /// For a shift, symbols or letters key: change face and say so. The highlight
    /// stays on the key that was pressed (or the one that took its place).
    pub fn switch_face(&mut self) -> bool {
        let (face, stay_on) = match self.key().kind {
            Kind::Shift if self.face == Face::Upper => (Face::Lower, Kind::Shift),
            Kind::Shift => (Face::Upper, Kind::Shift),
            Kind::Symbols => (Face::Symbols, Kind::Letters),
            Kind::Letters => (Face::Lower, Kind::Symbols),
            _ => return false,
        };
        self.face = face;
        self.point_to(stay_on);
        true
    }

    /// Move the highlight. False at an edge.
    pub fn step(&mut self, dir: Dir) -> bool {
        let rows = self.rows();
        let row = &rows[self.row];
        match dir {
            Dir::Left if self.index > 0 => self.index -= 1,
            Dir::Right if self.index + 1 < row.len() => self.index += 1,
            Dir::Up | Dir::Down => {
                let target = match dir {
                    Dir::Up if self.row > 0 => self.row - 1,
                    Dir::Down if self.row + 1 < rows.len() => self.row + 1,
                    _ => return false,
                };
                // Land on the key under the middle of this one (in half columns).
                let middle = starts(row)[self.index] * 2 + row[self.index].span;
                let under = starts(&rows[target]).iter().rposition(|start| middle >= start * 2).unwrap_or(0);
                (self.row, self.index) = (target, under);
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_of_every_face_is_ten_columns_wide() {
        for face in [Face::Lower, Face::Upper, Face::Symbols] {
            for row in rows(face) {
                assert_eq!(row.iter().map(|key| key.span).sum::<usize>(), COLUMNS, "{face:?}");
            }
        }
    }

    #[test]
    fn the_highlight_moves_as_the_keys_lie_and_stops_at_the_edges() {
        let mut k = Model::default();
        assert_eq!(k.key().label, "a");
        assert!(!k.step(Dir::Left) && !k.step(Dir::Up));
        assert!(k.step(Dir::Right) && k.step(Dir::Down));
        assert_eq!(k.key().label, "l");
        // Down the right-hand side to the wide keys, and back up from one.
        k.point_at(9);
        assert_eq!(k.key().label, "j");
        for _ in 0..4 {
            assert!(k.step(Dir::Down));
        }
        assert_eq!(k.key().kind, Kind::Submit);
        assert!(!k.step(Dir::Down) && !k.step(Dir::Right));
        assert!(k.step(Dir::Left));
        assert_eq!(k.key().kind, Kind::Delete);
        assert!(k.step(Dir::Up));
        assert_eq!(k.key().label, "8", "the key above the middle of delete");
        assert_eq!(k.flat(), 37);
    }

    #[test]
    fn shift_and_the_symbols_key_change_face_and_keep_their_place() {
        let mut k = Model::default();
        k.point_to(Kind::Shift);
        assert!(k.switch_face());
        assert_eq!((k.face, k.key().label.as_str()), (Face::Upper, "SHIFT"));
        assert_eq!(k.rows()[0][0].label, "A");
        assert!(k.switch_face());
        assert_eq!(k.face, Face::Lower);
        k.point_to(Kind::Symbols);
        assert!(k.switch_face());
        assert_eq!((k.face, k.key().kind), (Face::Symbols, Kind::Letters));
        assert!(k.switch_face());
        assert_eq!((k.face, k.key().kind), (Face::Lower, Kind::Symbols));
        k.point_at(0);
        assert!(!k.switch_face(), "a letter is not a face key");
    }
}
