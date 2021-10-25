extern crate ringbuf;

use std::{error, io};
use std::cell::RefCell;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use cpal::{BufferSize, Device, DevicesError, SampleRate};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::{event::{self, Event, KeyCode, KeyEvent}};
use tui::{
    backend::CrosstermBackend,
    layout::{Constraint, Corner, Direction, Layout},
    style::{Color, Modifier, Style},
    Terminal,
    text::{Span, Spans},
    widgets::{Block, Borders, List, ListItem},
};
use tui::widgets::ListState;
use ringbuf::RingBuffer;

mod stateful_list;

pub struct StatefulList<T> {
    pub state: ListState,
    pub items: Vec<T>,
}

struct App {
    input_devices: StatefulList<(Device, usize)>,
    output_devices: StatefulList<(Device, usize)>,
    active_panel_index: i8,
    factor: f32,
    // link_is_active: bool,
    // input_stream: Arc<Mutex<Option<cpal::Stream>>>,
    // output_stream: Option<cpal::Stream>,
}

impl App {
    fn new(input_devices: StatefulList<(Device, usize)>, output_devices: StatefulList<(Device, usize)>) -> App {
        App{
            input_devices,
            output_devices,
            active_panel_index: 0,
            factor: 1.0,
            // link_is_active: false,
            // input_stream: Arc::new(Mutex::new(None)),
            // output_stream: None,
        }
    }

    fn get_factor(&self) -> f32 {
        self.factor
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

    // fn increase_volume(&mut self) {
    //     self.factor += 10.0;
    //     if self.link_is_active {
    //         self.link_selected_devices();
    //     }
    // }
    //
    // fn decrease_volume(&mut self) {
    //     let new_factor = self.factor - 0.1;
    //     self.factor = if new_factor > 0.0 { new_factor } else { 0.0 };
    //     if self.link_is_active {
    //         self.link_selected_devices();
    //     }
    // }

    fn link_selected_devices(&self) -> Result<(), Box<dyn error::Error>> {
        let ring = RingBuffer::new(8192);
        let (mut producer, mut consumer) = ring.split();
        let get_factor = || {
            &self.factor
        };

        let input_data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for &sample in data {
                producer.push(sample * 1.0);
                // if producer.push(sample).is_err() {
                //     output_fell_behind = true;
                // }
            }
        };

        let output_data_fn = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut input_fell_behind = None;
            for sample in data {
                *sample = match consumer.pop() {
                    Some(s) => s,
                    None => {
                        eprintln!("Reading error");
                        input_fell_behind = Some("Reading error");
                        0.0
                    }
                };
            }
        };

        let input_device = self.input_devices.state.selected().map(|i| &self.input_devices.items[i].0).expect("No input device selected");
        // let output_device = self.output_devices.state.selected().map(|i| &self.output_devices.items[i].0).expect("No output device selected");
        let output_device = cpal::default_host().default_output_device().unwrap();

        // let input_config = cpal::StreamConfig{ channels: 2, sample_rate: SampleRate(44100), buffer_size: BufferSize::Default };
        let input_config = input_device.default_input_config().unwrap().into();
        let input_stream = input_device.build_input_stream(&input_config, input_data_fn, err_fn).unwrap();

        // if self.input_stream.lock().unwrap().deref().is_some() {
        //     // self.input_stream = None;
        //     let ptr = self.input_stream.lock().unwrap();
        //     *ptr = None;
        // }

        // if self.output_stream.is_some() {
        //     self.output_stream = None;
        // }

        // let output_config = cpal::StreamConfig{ channels: 2, sample_rate: SampleRate(48000), buffer_size: BufferSize::Default };
        let output_config = output_device.default_output_config().unwrap().into();
        let output_stream = output_device.build_output_stream(&output_config, output_data_fn, err_fn).unwrap();

        input_stream.play()?;
        output_stream.play()?;
        println!("Streams are connected");

        // for the purposes of this demo, leak these so that after returning the audio units will
        // keep running
        std::mem::forget(input_stream);
        std::mem::forget(output_stream);
        // self.input_stream.replace(Some(input_stream));
        // self.output_stream.replace(Some(output_stream));
        // self.link_is_active = true;
        // **self.input_stream = Some(input_stream);
        // self.input_stream.lock().unwrap().replace(input_stream);
        // self.output_stream = Some(output_stream);
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn error::Error>>{
    let host = cpal::default_host();
    let input_devices = host.input_devices()?;
    let output_devices = host.output_devices()?;

    // host.output_devices()?.for_each(|dev| {
    //     let configs = dev.supported_output_configs().unwrap();
    //     configs.for_each(|config| println!("Device: {}, config: {:?}", &dev.name().unwrap(), config))
    // }
    // );

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let l: StatefulList<(Device, usize)> = StatefulList::with_items(
        input_devices.enumerate().map(|(i, dev)| (dev, i)).collect()
    );

    let r: StatefulList<(Device, usize)> = StatefulList::with_items(
        output_devices.enumerate().map(|(i, dev)| (dev, i)).collect()
    );

    let mut app = App::new(l, r);

    loop {
        terminal.draw(|f| {
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
            f.render_stateful_widget(input_devices_widget, chunks[0], &mut app.input_devices.state);

            let right_items: Vec<ListItem> = make_devices_widget_items(&app.output_devices.items);

            let output_devices_widget = List::new(right_items).highlight_style(
                Style::default()
                    .bg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            );

            f.render_stateful_widget(output_devices_widget, chunks[1], &mut app.output_devices.state)
        });

        match event::read() {
            Ok(evt) => if let Event::Key(k) = evt {
                match k {
                    KeyEvent { code: KeyCode::Char('q'), .. } => {break; }
                    // KeyEvent { code: KeyCode::Char('+'), .. } => app.increase_volume(),
                    // KeyEvent { code: KeyCode::Char('-'), .. } => app.decrease_volume(),
                    KeyEvent { code: KeyCode::Down, .. } => app.active_panel().next(),
                    KeyEvent { code: KeyCode::Up, .. } => app.active_panel().previous(),
                    KeyEvent { code: KeyCode::Tab, .. } => app.next_panel(),
                    KeyEvent { code: KeyCode::Enter, .. } => app.link_selected_devices().unwrap(),
                    _ => {}
                }
            }
            Err(_) => {}
        }
    }

    terminal.clear()?;
    Ok(())
}

fn make_devices_widget_items(devices: &Vec<(Device, usize)>) -> Vec<ListItem> {
    let input_devices_list_style = Style::default().fg(Color::Black).bg(Color::White);
    devices.iter()
        .map(|(dev, i)|
            ListItem::new(dev.name().unwrap().to_string())
                .style(input_devices_list_style.clone())
        ).collect()
}

fn err_fn(err: cpal::StreamError) {
    eprintln!("an error occurred on stream: {:?}", err);
}
