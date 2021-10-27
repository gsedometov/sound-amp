extern crate ringbuf;

use std::sync::{mpsc, Arc, Mutex};
use std::{error, io, thread};
use std::io::Stdout;


use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Device, InputCallbackInfo, OutputCallbackInfo, SampleRate};
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use ringbuf::RingBuffer;
use tui::widgets::ListState;
use tui::{backend::CrosstermBackend, layout::{Constraint, Direction, Layout}, style::{Color, Modifier, Style}, widgets::{List, ListItem}, Terminal, Frame};

mod stateful_list;

pub struct StatefulList<T> {
    pub state: ListState,
    pub items: Vec<T>,
}

struct App {
    input_devices: StatefulList<(Device, usize)>,
    output_devices: StatefulList<(Device, usize)>,
    active_panel_index: u8,
}

impl App {
    fn new(
        input_devices: StatefulList<(Device, usize)>,
        output_devices: StatefulList<(Device, usize)>,
    ) -> App {
        App {
            input_devices,
            output_devices,
            active_panel_index: 0,
        }
    }

    fn active_panel(&mut self) -> &mut StatefulList<(Device, usize)> {
        if self.active_panel_index == 0 {
            &mut self.input_devices
        } else {
            &mut self.output_devices
        }
    }

    fn next_panel(&mut self) {
        self.active_panel_index = (self.active_panel_index + 1) % 2
    }
}

fn main() -> Result<(), Box<dyn error::Error>> {
    let host = cpal::default_host();
    let input_devices = host.input_devices()?;
    let output_devices = host.output_devices()?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let l: StatefulList<(Device, usize)> =
        StatefulList::with_items(input_devices.enumerate().map(|(i, dev)| (dev, i)).collect());

    let r: StatefulList<(Device, usize)> = StatefulList::with_items(
        output_devices
            .enumerate()
            .map(|(i, dev)| (dev, i))
            .collect(),
    );

    let mut app = App::new(l, r);
    let player_channel = setup_stream();
    loop {
        terminal.draw(|f| draw_tui(f, &mut app));
        match event::read() {
            Ok(evt) => {
                if let Event::Key(k) = evt {
                    match k {
                        KeyEvent {
                            code: KeyCode::Char('q'),
                            ..
                        } => {
                            break;
                        }
                        KeyEvent {
                            code: KeyCode::Char('+'),
                            ..
                        } => {
                            player_channel.send(PlayerCommand::IncreaseVolume(1.0));
                        }
                        KeyEvent {
                            code: KeyCode::Char('-'),
                            ..
                        } => {
                            player_channel.send(PlayerCommand::IncreaseVolume(-1.0));
                        }
                        KeyEvent {
                            code: KeyCode::Down,
                            ..
                        } => app.active_panel().next(),
                        KeyEvent {
                            code: KeyCode::Up, ..
                        } => app.active_panel().previous(),
                        KeyEvent {
                            code: KeyCode::Tab, ..
                        } => app.next_panel(),
                        KeyEvent {
                            code: KeyCode::Enter,
                            ..
                        } => {
                            player_channel.send(PlayerCommand::Start(
                                app.input_devices.state.selected().unwrap(),
                            ));
                        }
                        _ => {}
                    }
                }
            }
            Err(_) => {}
        }
    }

    terminal.clear()?;
    Ok(())
}

fn draw_tui(f: &mut Frame<CrosstermBackend<Stdout>>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(f.size());

    let left_items: Vec<ListItem> = make_devices_widget_items(&app.input_devices.items);

    let input_devices_widget = List::new(left_items).highlight_style(
        Style::default()
            .bg(Color::LightGreen)
            .add_modifier(Modifier::BOLD),
    );
    f.render_stateful_widget(
        input_devices_widget,
        chunks[0],
        &mut app.input_devices.state,
    );

    let right_items: Vec<ListItem> = make_devices_widget_items(&app.output_devices.items);

    let output_devices_widget = List::new(right_items).highlight_style(
        Style::default()
            .bg(Color::LightGreen)
            .add_modifier(Modifier::BOLD),
    );

    f.render_stateful_widget(
        output_devices_widget,
        chunks[1],
        &mut app.output_devices.state,
    )
}

fn make_devices_widget_items(devices: &Vec<(Device, usize)>) -> Vec<ListItem> {
    let input_devices_list_style = Style::default().fg(Color::Black).bg(Color::White);
    devices
        .iter()
        .map(|(dev, _i)| {
            ListItem::new(dev.name().unwrap()).style(input_devices_list_style)
        })
        .collect()
}

enum PlayerCommand {
    Start(usize),
    IncreaseVolume(f32),
}

fn setup_stream() -> mpsc::Sender<PlayerCommand> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut link: Vec<cpal::Stream> = vec![];
        let volume_factor = Arc::new(Mutex::new(1f32));
        let command_handler = |command: PlayerCommand| {
            match command {
                PlayerCommand::Start(input_device_i) => {
                    link = create_link(input_device_i, &volume_factor);
                }
                PlayerCommand::IncreaseVolume(amount) => {
                    *volume_factor.lock().unwrap() += amount;
                }
            }
        };
        rx.iter().for_each(command_handler);
    });
    tx
}

fn create_link(
    input_device_id: usize,
    volume_factor: &Arc<Mutex<f32>>,
) -> Vec<cpal::Stream> {
    let host = cpal::default_host();
    let output_device = host
        .default_output_device()
        .expect("Failed to get default output device");
    println!("Sound device: {}", output_device.name().unwrap());

    let format = output_device
        .default_output_config()
        .expect("Failed to get default output format");

    println!("Format: {:?}", format);
    let ring: RingBuffer<f32> = RingBuffer::new(48000);
    let (mut producer, mut consumer) = ring.split();
    let input_device = &host.input_devices().unwrap().collect::<Vec<Device>>()[input_device_id];
    let input_stream = {
        let factor = Arc::clone(volume_factor);
        let data_callback = move |data: &[f32], _: &InputCallbackInfo| {
            let factor_value = *factor.lock().unwrap();
            for &sample in data {
                producer.push(sample * factor_value);
            };
        };
        let s = input_device
            .build_input_stream(
                &input_device.default_input_config().unwrap().into(),
                data_callback,
                err_fn,
            )
            .expect("Cannot create input stream");
        s.play().expect("Cannot start input stream");
        s
    };
    let output_stream = {
        let data_callback = move |data: &mut [f32], _: &OutputCallbackInfo| {
            for sample in data {
                *sample = consumer.pop().unwrap_or(0.0);
            }
        };
        let s = output_device
            .build_output_stream(
                &output_device.default_output_config().unwrap().into(),
                data_callback,
                err_fn,
            )
            .expect("Cannot create output stream");
        s.play().expect("Cannot start output stream");
        s
    };
    vec![input_stream, output_stream]
}

fn err_fn(err: cpal::StreamError) {
    eprintln!("an error occurred on stream: {:?}", err);
}
