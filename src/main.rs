use async_std::{
    io::BufReader,
    net::{TcpListener, TcpStream},
    prelude::*,
    task,
};
use std::{env, io, thread};

pub mod util;

use crate::util::event::{Event, Events};
use async_std::sync::{Arc, Mutex};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::error::Error;
use termion::{event::Key, raw::IntoRawMode};
use tui::layout::{Alignment, Constraint, Direction, Layout};
use tui::style::{Color, Modifier, Style};
use tui::Terminal;
use tui::{
    backend::TermionBackend,
    text::Spans,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};

extern crate jsonxf;

#[derive(Debug, Deserialize)]
struct BacktraceItem {
    file: String,
    line: i64,
    function: String,
}

#[derive(Debug, Deserialize)]
struct DebugEntry {
    label: String,
    time: String,
    data: HashMap<String, Value>,
    backtrace: Vec<BacktraceItem>,
}

// Table holding all the logging values.
pub struct StatefulTable {
    state: TableState,
    items: Vec<DebugEntry>,
}

impl StatefulTable {
    fn new() -> StatefulTable {
        StatefulTable {
            state: TableState::default(),
            items: Vec::new(),
        }
    }

    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let stdout = io::stdout().into_raw_mode()?;
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let events = Events::new();

    let mutex_table = Arc::new(Mutex::new(StatefulTable::new()));

    let args: Vec<String> = env::args().collect();

    let mut port: i32 = 9337;

    if args.len() == 2 {
        port = args[1].parse().unwrap();
    }

    // Thread to listen for incoming connections.
    let thread_table = Arc::clone(&mutex_table);
    thread::spawn(move || {
        task::block_on(handle_tcp(port, thread_table)).unwrap();
    });

    loop {
        terminal
            .draw(|f| {
                let table: &mut StatefulTable = &mut mutex_table.try_lock().unwrap();
                let layout = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(25), Constraint::Percentage(75)].as_ref())
                    .split(f.size());

                let selected_style = Style::default().add_modifier(Modifier::REVERSED);
                let normal_style = Style::default().bg(Color::Blue);
                let header_cells = vec![Cell::from("Entry")];
                let header = Row::new(header_cells).style(normal_style).bottom_margin(1);
                let rows = table.items.iter().map(|item| {
                    let cells = vec![Cell::from(item.label.as_str())];
                    Row::new(cells)
                });
                let table_widget = Table::new(rows)
                    .header(header)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Entries")
                            .style(Style::default().bg(Color::Black)),
                    )
                    .highlight_style(selected_style)
                    .highlight_symbol("> ")
                    .widths(&[
                        Constraint::Percentage(100),
                        Constraint::Length(30),
                        Constraint::Max(10),
                    ]);
                f.render_stateful_widget(table_widget, layout[0], &mut table.state);

                let detail_rects = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(70), Constraint::Percentage(30)].as_ref())
                    .split(layout[1]);

                let detail_widget = Block::default()
                    .borders(Borders::ALL)
                    .title("Details")
                    .style(Style::default().bg(Color::Black));

                let backtrace_widget = Block::default()
                    .borders(Borders::ALL)
                    .title("Backtrace")
                    .style(Style::default().bg(Color::Black));

                if table.items.len() > 0 {
                    // Set the last item to be selected if no selection is active yet.
                    let item = match table.state.selected() {
                        None => {
                            let last_index = table.items.len() - 1;
                            table.state.select(Option::from(last_index));
                            table.items.get(last_index).unwrap()
                        }
                        Some(index) => table.items.get(index).unwrap(),
                    };

                    let text: Vec<Spans> = build_paragraph_for_item(&item);

                    let details = Paragraph::new(text)
                        .block(detail_widget)
                        .wrap(Wrap { trim: true })
                        .alignment(Alignment::Left);
                    f.render_widget(details, detail_rects[0]);

                    // Render the backtrace.
                    let rows = item.backtrace.iter().map(|backtrace_item| {
                        let cells = vec![
                            Cell::from(backtrace_item.file.as_str()),
                            Cell::from(backtrace_item.line.to_string()),
                            Cell::from(backtrace_item.function.as_str()),
                        ];
                        Row::new(cells)
                    });
                    let heading = Row::new(vec![
                        Cell::from("File"),
                        Cell::from("Line"),
                        Cell::from("Calling function"),
                    ])
                    .style(normal_style)
                    .bottom_margin(1);

                    let backtrace_table = Table::new(rows)
                        .header(heading)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title("Backtrace")
                                .style(Style::default().bg(Color::Black)),
                        )
                        .highlight_style(selected_style)
                        .highlight_symbol("> ")
                        .widths(&[
                            Constraint::Percentage(50),
                            Constraint::Percentage(10),
                            Constraint::Percentage(40),
                        ]);

                    f.render_widget(backtrace_table, detail_rects[1]);
                } else {
                    f.render_widget(detail_widget, detail_rects[0]);
                    f.render_widget(backtrace_widget, detail_rects[1]);
                }
            })
            .unwrap();

        if let Event::Input(input) = events.next()? {
            match input {
                Key::Esc => {
                    // Quit the loop and terminate the application.
                    break;
                }
                Key::Char('j') => {
                    let table: &mut StatefulTable = &mut mutex_table.try_lock().unwrap();
                    table.next();
                }
                Key::Char('k') => {
                    let table: &mut StatefulTable = &mut mutex_table.try_lock().unwrap();
                    table.previous();
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn build_paragraph_for_item(item: &DebugEntry) -> Vec<Spans> {
    let mut result: Vec<Spans> = vec![];

    let text = build_text_vec_from_hashmap(item.data.clone(), 0);

    result.push(Spans::from(format!("Logged on: {}", item.time)));
    result.push(Spans::from(format!("")));

    for text_node in text {
        result.push(Spans::from(format!("{}", text_node)));
    }

    result
}

fn build_text_vec_from_hashmap(map: HashMap<String, Value>, level: usize) -> Vec<String> {
    let mut result: Vec<String> = vec![];

    let indent = "-".repeat(level * 2);

    for (val_type, value) in &map {
        match value {
            Value::Null => result.push(format!("{} {}", indent, val_type)),
            Value::Bool(value) => result.push(format!("{} {} {}", indent, val_type, value)),
            Value::Number(value) => result.push(format!("{} {} {}", indent, val_type, value)),
            Value::String(value) => result.push(format!("{} {} {}", indent, val_type, value)),
            Value::Array(_) => { /* No need to handle */ }
            Value::Object(value) => {
                result.extend_from_slice(&build_text_vec_from_object(value.clone(), level + 1, val_type.to_string()))
            }
        }
    }

    result
}

fn build_text_vec_from_object(value: Map<String, Value>, level: usize, label: String) -> Vec<String> {
    let mut result: Vec<String> = vec![];
    let indent = "-".repeat(level * 2);
    for (item_key, item_value) in &value {
        match item_value {
            Value::Null => result.push(format!("{} {} NULL", indent, item_key)),
            Value::Bool(value) => result.push(format!("{} {} {}", indent, item_key, value)),
            Value::Number(value) => result.push(format!("{} {} {}", indent, item_key, value)),
            Value::String(value) => result.push(format!("{} {} {}", indent, item_key, value)),
            Value::Array(_) => { /* No need to handle */ }
            Value::Object(value) => {
                // Otherwise we duplicate this information. To consider: Would this break values
                // with the name "array"?
                if item_key != "array" {
                    result.push(format!("{} {} ({})", indent, item_key, label));
                }
                result.extend_from_slice(&build_text_vec_from_object(value.clone(), level + 1, item_key.to_string()))
            }
        };
    }

    result
}

type SomeResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn handle_tcp(port: i32, table: Arc<Mutex<StatefulTable>>) -> SomeResult<()> {
    let listener: TcpListener = TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    let mut incoming = listener.incoming();
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        let handle = task::spawn(connection_loop(stream, table.clone()));
        handle.await?
    }

    Ok(())
}

async fn read(stream: &Arc<TcpStream>) -> String {
    let stream = stream.clone();

    let mut reader = BufReader::new(stream.as_ref());

    let mut content: Vec<u8> = Vec::new();

    // The second read will read everything until the next null byte which is the end of the
    // message.
    let result = reader.read_until(b'\0', &mut content).await;

    match result {
        Ok(_) => String::from_utf8_lossy(&content).to_string(),
        Err(_) => String::from("error"),
    }
}

async fn connection_loop(stream: TcpStream, table: Arc<Mutex<StatefulTable>>) -> SomeResult<()> {
    let stream = Arc::new(stream);

    let value = read(&stream).await;

    let debug_entry: DebugEntry = serde_json::from_str(value.as_str()).unwrap();

    table.try_lock().unwrap().items.insert(0, debug_entry);

    Ok(())
}
