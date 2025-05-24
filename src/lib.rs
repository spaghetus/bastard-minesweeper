#![warn(clippy::pedantic)]

use std::{
    collections::HashMap,
    ops::{Deref, DerefMut, Range, RangeInclusive, Rem},
    sync::Arc,
    time::{Duration, Instant},
};

use indicatif::{ProgressBar, ProgressIterator, ProgressStyle};
use itertools::Itertools;
use ndarray::Array2;
use rand::{Rng, distr::slice::Choose, rng};

#[derive(Clone, Copy, Debug)]
pub enum Cell {
    Quantum(Option<bool>),
    Discovered(Option<u8>),
    Concrete(bool),
}

impl Default for Cell {
    fn default() -> Self {
        Cell::Quantum(None)
    }
}

impl Cell {
    #[must_use]
    pub fn bomb_count(&self) -> RangeInclusive<u8> {
        match self {
            Cell::Quantum(None) => 0..=1,
            Cell::Quantum(Some(b)) | Cell::Concrete(b) => u8::from(*b)..=u8::from(*b),
            Cell::Discovered(_) => 0..=0,
        }
    }

    #[must_use]
    pub fn is_bomb(&self) -> bool {
        match self {
            Cell::Quantum(Some(b)) | Cell::Concrete(b) => *b,
            Cell::Quantum(None) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Board(Array2<Cell>);

impl Deref for Board {
    type Target = Array2<Cell>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Board {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Board {
    #[must_use]
    pub fn new(w: usize, h: usize) -> Self {
        Self(Array2::default((w, h)))
    }
    pub fn points(&self) -> impl Iterator<Item = (usize, usize)> {
        let (width, height) = self.dim();
        (0..width).cartesian_product(0..height)
    }
    pub fn neighbors(&self, x: usize, y: usize) -> impl Iterator<Item = (usize, usize, &Cell)> {
        (-1..=1isize)
            .cartesian_product(-1..=1isize)
            .filter(|p| *p != (0, 0))
            .filter_map(move |(dx, dy)| {
                x.checked_add_signed(dx)
                    .and_then(|x| Some((x, y.checked_add_signed(dy)?)))
            })
            .filter_map(|(x, y)| Some((x, y, self.get((x, y))?)))
    }
    /// Check whether assigning a particular value to a cell would violate any existing discovered cells
    #[must_use]
    pub fn assignment_is_legal(&self, x: usize, y: usize, value: bool) -> bool {
        let new_value = u8::from(value);
        let current_value = self[(x, y)].bomb_count();
        self.neighbors(x, y)
            .filter_map(|(x, y, c)| {
                if let Cell::Discovered(Some(n)) = c {
                    Some((x, y, *n))
                } else {
                    None
                }
            })
            .all(|(x, y, wants_bombs)| {
                let range = self.count_neighboring_bombs(x, y);
                let range = (range.start() - current_value.start() + new_value)
                    ..=(range.end() - current_value.end() + new_value);
                range.contains(&wants_bombs)
            })
    }
    /// Count the number of bombs neighboring a cell
    #[must_use]
    pub fn count_neighboring_bombs(&self, x: usize, y: usize) -> RangeInclusive<u8> {
        self.neighbors(x, y)
            .map(|(_, _, c)| c.bomb_count())
            .fold(0..=0, |acc, el| {
                (acc.start() + el.start())..=(acc.end() + el.end())
            })
    }
    /// Clear a cell, returning false if this cell was a bomb
    pub fn clear_cell(&mut self, x: usize, y: usize) -> bool {
        let c = self[(x, y)];

        match c {
            Cell::Quantum(Some(false)) | Cell::Concrete(false) => {
                self[(x, y)] = Cell::Discovered(None);
                true
            }
            Cell::Discovered(_) => true,
            _ => false,
        }
    }
    /// Find the values for all discovered cells
    #[must_use]
    pub fn find_discovered_counts(&self) -> Vec<((usize, usize), u8)> {
        let (width, height) = self.dim();
        (0..width)
            .cartesian_product(0..height)
            .filter_map(|(x, y)| {
                if !matches!(self[(x, y)], Cell::Discovered(None)) {
                    return None;
                }
                let range = self.count_neighboring_bombs(x, y);
                debug_assert_eq!(range.start(), range.end());
                Some(((x, y), *range.start()))
            })
            .collect()
    }
    /// Fill in discovered cells with their counts
    pub fn fill_discovered(&mut self) {
        let (width, height) = self.dim();
        (0..width)
            .cartesian_product(0..height)
            .filter_map(|(x, y)| {
                if !matches!(self[(x, y)], Cell::Discovered(_)) {
                    return None;
                }
                let range = self.count_neighboring_bombs(x, y);
                debug_assert_eq!(range.start(), range.end());
                Some(((x, y), *range.start()))
            })
            .collect_vec()
            .into_iter()
            .for_each(|(c, v)| {
                if let Cell::Discovered(Some(r)) = self[c] {
                    debug_assert_eq!(v, r);
                }
                self[c] = Cell::Discovered(Some(v));
            });
    }
    /// Collapse all quantum cells
    #[allow(clippy::too_many_lines, clippy::missing_panics_doc)]
    pub fn collapse(&mut self, mut max_bombs: usize, allowed_range: Option<Range<(usize, usize)>>) {
        eprintln!("Collapsing...");
        let (width, height) = self.dim();
        let allowed_range = allowed_range.unwrap_or((0, 0)..(width, height));
        let mut quantum_cells = (0..width)
            .cartesian_product(0..height)
            .filter(|(x, y)| {
                matches!(self[(*x, *y)], Cell::Quantum(_))
                    && self
                        .neighbors(*x, *y)
                        .any(|(_, _, n)| matches!(n, Cell::Discovered(_)))
            })
            .collect_vec();
        {
            let mut true_check_board = self.clone();
            true_check_board.iter_mut().for_each(|c| {
                if matches!(c, Cell::Quantum(Some(false))) {
                    *c = Cell::Quantum(None);
                }
            });
            quantum_cells.retain(|(x, y)| match self[(*x, *y)] {
                Cell::Quantum(Some(true)) => true_check_board.assignment_is_legal(*x, *y, false),
                Cell::Quantum(_) => true,
                _ => false,
            });
            let mut false_check_board = self.clone();
            false_check_board.iter_mut().for_each(|c| {
                if matches!(c, Cell::Quantum(Some(true))) {
                    *c = Cell::Quantum(None);
                }
            });
            quantum_cells.retain(|(x, y)| match self[(*x, *y)] {
                Cell::Quantum(Some(false)) => false_check_board.assignment_is_legal(*x, *y, true),
                Cell::Quantum(_) => true,
                _ => false,
            });

            quantum_cells.retain(|(x, y)| {
                (allowed_range.start.0..allowed_range.end.0).contains(x)
                    && (allowed_range.start.1..allowed_range.end.1).contains(y)
            });
        }

        quantum_cells
            .iter()
            .for_each(|p| self[*p] = Cell::Quantum(None));

        max_bombs = max_bombs.saturating_sub(
            self.iter()
                .filter(|c| matches!(c, Cell::Concrete(true) | Cell::Quantum(Some(true))))
                .count(),
        );

        if max_bombs == 0 {
            quantum_cells
                .iter()
                .for_each(|c| self[*c] = Cell::Quantum(Some(false)));
            eprintln!("run out of bombs");
            return;
        }
        if quantum_cells.is_empty() {
            eprintln!("can't assign any cells");
            return;
        }
        let mut rng = rng();
        quantum_cells.sort_by_key(|(x, y)| x + y);
        quantum_cells
            .iter()
            .for_each(|c| self[*c] = Cell::Quantum(None));
        eprintln!(
            "{} quantum cells, {max_bombs} bombs to place",
            quantum_cells.len()
        );
        let progress = ProgressBar::no_length().with_style(
            ProgressStyle::default_spinner()
                .template("{spinner} {per_sec}")
                .unwrap(),
        );
        progress.enable_steady_tick(Duration::from_millis(100));
        let began = Instant::now();
        let states = self
            .clone()
            .collapse_inner(Arc::new(Cons::Empty), 0, &quantum_cells, max_bombs)
            .into_iter()
            .flatten()
            .progress_with(progress)
            .map(|s| {
                let mut s = &s;
                let mut v = std::iter::from_fn(move || {
                    if let Cons::Cell(b, next) = &**s {
                        s = next;
                        Some(*b)
                    } else {
                        None
                    }
                })
                .collect_vec();
                v.reverse();
                v
            })
            .collect_vec();
        eprintln!(
            "{} possible states in {}s",
            states.len(),
            began.elapsed().as_secs_f32()
        );
        if !states.is_empty() {
            let began = Instant::now();
            let state_counts = (&mut rng)
                .sample_iter(Choose::new(states.as_slice()).unwrap())
                .take(states.len())
                .take_while(|_| began.elapsed() < Duration::from_secs(2))
                .map(|s| {
                    s.iter()
                        .zip(&quantum_cells)
                        .map(|(b, (x, y))| ((*x, *y), b))
                        .for_each(|(c, b)| self[c] = Cell::Quantum(Some(*b)));
                    (self.find_discovered_counts(), s)
                })
                .fold(HashMap::new(), |mut acc, (numbers, quanta)| {
                    acc.entry(numbers).or_insert((0usize, quanta)).0 += 1;
                    acc
                });
            eprintln!(
                "{} unique sets found in {}s of sampling",
                state_counts.len(),
                began.elapsed().as_secs_f32()
            );
            if let Some((_, (amt, quanta))) = state_counts.iter().max_by_key(|(_, count)| **count) {
                eprintln!("Chose a state with {amt} possible bomb placements");
                // best_state
                //     .iter()
                //     .for_each(|(c, v)| self[*c] = Cell::Discovered(Some(*v)));
                quantum_cells
                    .iter()
                    .zip(quanta.iter())
                    .for_each(|(c, v)| self[*c] = Cell::Quantum(Some(*v)));
            }
        }
    }
    fn collapse_inner<'a>(
        self,
        list: Arc<Cons<bool>>,
        depth: usize,
        cells: &'a [(usize, usize)],
        max_bombs: usize,
    ) -> Option<Box<dyn Iterator<Item = Arc<Cons<bool>>> + Send + 'a>> {
        let [(x, y), rest @ ..] = cells else {
            if matches!(*list, Cons::Cell(_, _)) {
                return Some(Box::new(std::iter::once(list)));
            }
            return None;
        };
        let x = *x;
        let y = *y;
        let left_is_legal = max_bombs > 0 && self.assignment_is_legal(x, y, true);
        let right_is_legal = self.assignment_is_legal(x, y, false);
        let depth_increase = usize::from(left_is_legal && right_is_legal);
        let left = {
            let list = list.clone();
            let mut board = self.clone();
            move || {
                if left_is_legal {
                    board[(x, y)] = Cell::Quantum(Some(true));
                    board
                        .collapse_inner(
                            Arc::new(Cons::Cell(true, list)),
                            depth + depth_increase,
                            rest,
                            max_bombs - 1,
                        )
                        .map(|i| Box::new(i) as Box<dyn Iterator<Item = Arc<Cons<bool>>> + Send>)
                } else {
                    None
                }
            }
        };
        let right = {
            let list = list.clone();
            let mut board = self.clone();
            move || {
                if right_is_legal {
                    board[(x, y)] = Cell::Quantum(Some(false));
                    board
                        .collapse_inner(
                            Arc::new(Cons::Cell(false, list)),
                            depth + depth_increase,
                            rest,
                            max_bombs,
                        )
                        .map(|i| Box::new(i) as Box<dyn Iterator<Item = Arc<Cons<bool>>> + Send>)
                } else {
                    None
                }
            }
        };

        match (left_is_legal, right_is_legal) {
            (true, true) => {
                if depth.rem(18) == 6 {
                    let (left, right) = rayon::join(left, right);
                    Some(Box::new(left.into_iter().chain(right).flatten()))
                } else {
                    Some(Box::new(
                        [
                            Box::new(left) as Box<dyn FnOnce() -> _ + Send>,
                            Box::new(right) as Box<dyn FnOnce() -> _ + Send>,
                        ]
                        .into_iter()
                        .filter_map(|i| i())
                        .flatten(),
                    ))
                }
            }
            (true, false) => left(),
            (false, true) => right(),
            (false, false) => None,
        }

        // match (left, right) {
        //     (Some(left), Some(right)) => Some(Box::new(left.chain(right))),
        //     (Some(one), None) | (None, Some(one)) => Some(one),
        //     (None, None) => None,
        // }
    }
}

#[derive(Debug)]
enum Cons<T> {
    Empty,
    Cell(T, Arc<Self>),
}

// pub enum Board {
//     Quad([[Arc<Board>; 2]; 2]),
//     Concrete(Array2<Cell>),
// }

// impl Board {
//     pub fn size(&self) -> (usize, usize) {
//         match self {
//             Board::Quad([[tl, tr], [bl, br]]) => {
//                 let tl = tl.size();
//                 let tr = tr.size();
//                 let bl = bl.size();
//                 let br = br.size();
//                 let top_width = tl.0 + tr.0;
//                 let bottom_width = bl.0 + br.0;
//                 debug_assert_eq!(top_width, bottom_width);
//                 let left_height = tl.1 + bl.1;
//                 let right_height = tr.1 + br.1;
//                 debug_assert_eq!(left_height, right_height);
//                 (top_width, left_height)
//             }
//             Board::Concrete(array) => array.dim(),
//         }
//     }

//     pub fn call_descend(&self, )

//     pub fn assignment_is_legal(&self, x: usize, y: usize, value: bool) -> bool {
//         todo!()
//     }

//     pub fn collapse(self: Arc<Self>, maximize: impl Fn(&Self) -> f64) -> Arc<Self> {
//         match *self {
//             Board::Quad(_) => todo!(),
//             Board::Concrete(contents) => {}
//         }
//     }
// }

// impl Index<(usize, usize)> for Board {
//     type Output = Cell;

//     fn index(&self, index: (usize, usize)) -> &Self::Output {
//         match self {
//             Board::Quad(_) => todo!(),
//             Board::Concrete(array) => &array[index],
//         }
//     }
// }

// impl IndexMut<(usize, usize)> for Board {
//     type Output = Cell;

//     fn index_mut(&mut self, index: (usize, usize)) -> &mut Self::Output {
//         todo!()
//     }
// }
