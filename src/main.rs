#![warn(clippy::pedantic)]

use std::{collections::HashSet, thread::JoinHandle, usize};

use bastard_minesweeper::{Board, Cell};
use clap::Parser;
use eframe::{
    NativeOptions,
    egui::{CentralPanel, TopBottomPanel},
};
use egui_extras::{Column, TableBuilder};
use itertools::Itertools;
use rand::{Rng, rng};

#[derive(Parser)]
struct Args {
    #[arg(short, long, default_value = "10")]
    pub width: usize,
    #[arg(short, long, default_value = "10")]
    pub height: usize,
    /// Maximum bombs
    #[arg(short, long, default_value = "10")]
    pub max_bombs: usize,
    /// Bastard mode: Use quantum cells to make the game as annoying as possible
    #[arg(short, long)]
    pub bastard: bool,
}

fn main() {
    let Args {
        width,
        height,
        max_bombs,
        bastard,
    } = Args::parse();

    let mut board = Board::new(width, height);

    if !(bastard) {
        let mut rng = rng();
        let mut bombs_to_place = max_bombs;
        for (x, y) in (0..width).cartesian_product(0..height) {
            board[(x, y)] = Cell::Concrete(false);
        }
        while bombs_to_place > 0 {
            let x = rng.random_range(0..width);
            let y = rng.random_range(0..height);
            if !board[(x, y)].is_bomb() {
                board[(x, y)] = Cell::Concrete(true);
                bombs_to_place -= 1;
            }
        }
    }

    let app = App {
        board,
        worker: None,
        max_bombs,
        bastard,
        first_click: true,
        win: false,
        lose: None,
        cheat: false,
        flags: HashSet::new(),
    };

    eframe::run_native(
        if bastard {
            "Bastard Minesweeper"
        } else {
            "Minesweeper"
        },
        NativeOptions::default(),
        Box::new(move |_| Ok(Box::new(app))),
    )
    .unwrap();
}

#[allow(clippy::struct_excessive_bools)]
struct App {
    pub board: Board,
    pub worker: Option<JoinHandle<Board>>,
    pub max_bombs: usize,
    pub bastard: bool,
    pub first_click: bool,
    pub win: bool,
    pub cheat: bool,
    pub lose: Option<(usize, usize)>,
    pub flags: HashSet<(usize, usize)>,
}

impl eframe::App for App {
    #[allow(clippy::too_many_lines)]
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        if self.board.iter().all(|c| {
            matches!(
                c,
                Cell::Quantum(Some(true)) | Cell::Discovered(_) | Cell::Concrete(true)
            )
        }) {
            self.win = true;
        }
        // Join worker if we have one
        if let Some(worker) = std::mem::take(&mut self.worker) {
            if worker.is_finished() {
                self.worker = None;
                self.board = worker.join().unwrap();
            } else {
                self.worker = Some(worker);
                ctx.request_repaint();
            }
        }
        if self.worker.is_none() {
            let clearable_cells = self
                .board
                .points()
                .filter(|p| matches!(self.board[*p], Cell::Discovered(Some(0))))
                .flat_map(|(x, y)| {
                    self.board
                        .neighbors(x, y)
                        .map(|(x, y, _)| (x, y))
                        .filter(|p| matches!(self.board[*p], Cell::Quantum(_) | Cell::Concrete(_)))
                        .collect::<Vec<_>>()
                })
                .collect::<HashSet<_>>();
            if !clearable_cells.is_empty() {
                let allowed_range =
                    clearable_cells
                        .iter()
                        .fold((usize::MAX, usize::MAX)..(0, 0), |acc, el| {
                            (
                                acc.start.0.min(el.0.saturating_sub(2)),
                                acc.start.1.min(el.1.saturating_sub(2)),
                            )
                                ..(acc.end.0.max(el.0 + 3), acc.end.1.max(el.1 + 3))
                        });
                for (x, y) in clearable_cells {
                    self.board.clear_cell(x, y);
                }
                let mut new_board = self.board.clone();
                let bastard = self.bastard;
                let max_bombs = self.max_bombs;
                self.worker = Some(std::thread::spawn(move || {
                    if bastard {
                        while new_board
                            .iter()
                            .any(|c| matches!(c, Cell::Discovered(None)))
                        {
                            new_board.collapse(max_bombs, Some(allowed_range.clone()));
                            new_board.fill_discovered();
                        }
                    } else {
                        new_board.fill_discovered();
                    }
                    new_board
                }));
            }
        }
        TopBottomPanel::top("status").show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                if self.worker.is_some() {
                    ui.spinner();
                    ui.label("Busy");
                } else {
                    ui.label("Idle");
                }
                ui.separator();
                ui.checkbox(&mut self.cheat, "Cheat");
                if self.lose.is_some() {
                    ui.separator();
                    ui.label("You lose!");
                } else if self.win {
                    ui.separator();
                    ui.label("You win!");
                }
            });
        });
        CentralPanel::default().show(ctx, |ui| {
            let (width, height) = self.board.dim();
            TableBuilder::new(ui)
                .columns(Column::exact(16.), width)
                .body(|body| {
                    body.rows(16., height, |mut row| {
                        let y = row.index();
                        for x in 0..width {
                            let cell = self.board[(x, y)];
                            row.col(|ui| match cell {
                                Cell::Discovered(Some(n)) => {
                                    ui.label(n.to_string());
                                }
                                Cell::Quantum(_) | Cell::Concrete(_)
                                    if self.lose.is_none() && !self.win =>
                                {
                                    if self.flags.contains(&(x, y)) {
                                        if ui.button("F").secondary_clicked() {
                                            self.flags.remove(&(x, y));
                                        }
                                    } else {
                                        let button = ui.button(match cell {
                                            Cell::Quantum(Some(true)) | Cell::Concrete(true)
                                                if self.cheat =>
                                            {
                                                "B"
                                            }
                                            _ => " ",
                                        });
                                        if self.worker.is_none() && button.clicked() {
                                            if self.first_click {
                                                if self.bastard {
                                                    for dy in -2..=2 {
                                                        let y = y.saturating_add_signed(dy);
                                                        for dx in -2..=2 {
                                                            let x = x.saturating_add_signed(dx);
                                                            let Some(cell) =
                                                                self.board.get_mut((x, y))
                                                            else {
                                                                continue;
                                                            };
                                                            *cell = Cell::Discovered(None);
                                                        }
                                                    }
                                                } else {
                                                    self.board[(x, y)] = Cell::Discovered(None);
                                                }
                                            }
                                            if !self.board.clear_cell(x, y) {
                                                self.lose = Some((x, y));
                                                println!("Lose!");
                                                return;
                                            }
                                            let mut new_board = self.board.clone();
                                            let bastard = self.bastard;
                                            let max_bombs =
                                                if self.first_click { 8 } else { self.max_bombs };
                                            self.worker = Some(std::thread::spawn(move || {
                                                if bastard {
                                                    while new_board.iter().any(|c| {
                                                        matches!(c, Cell::Discovered(None))
                                                    }) {
                                                        new_board.collapse(
                                                            max_bombs,
                                                            Some(
                                                                (
                                                                    x.saturating_sub(5),
                                                                    y.saturating_sub(5),
                                                                )
                                                                    ..(x + 5, y + 5),
                                                            ),
                                                        );
                                                        new_board.fill_discovered();
                                                    }
                                                } else {
                                                    new_board.fill_discovered();
                                                }
                                                new_board
                                            }));
                                            self.first_click = false;
                                        }
                                        if button.secondary_clicked() {
                                            self.flags.insert((x, y));
                                        }
                                    }
                                }
                                Cell::Quantum(Some(b)) | Cell::Concrete(b) => {
                                    ui.label(if b {
                                        if self.lose == Some((x, y)) { "B" } else { "b" }
                                    } else {
                                        " "
                                    });
                                }
                                _ => {
                                    ui.label("?");
                                }
                            });
                        }
                    });
                });
        });
    }
}
