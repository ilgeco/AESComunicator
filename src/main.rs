#![feature(iter_advance_by)]
#![feature(vec_into_raw_parts)]

use std::{
    collections::VecDeque,
    env,
    io::{Read, Write},
    process::exit,
    time::Duration,
};

use serialport::TTYPort;

use crate::{actions::FunctorRes, serialcomunicator::SerialComunicator};

mod actions;
mod serialcomunicator;

fn main() {
    // Prints each argument on a separate line
    let mut args = env::args();
    let mut buffer = [0_u8; 16384];

    let tty_string = args.nth(1).unwrap_or("/dev/ttyUSB0".to_string());
    let tty_baud = u32::from_str_radix(&args.next().unwrap_or("115200".into()), 10).unwrap();
    let mut port = serialport::new(&tty_string, tty_baud)
        .timeout(Duration::from_secs(60))
        .open_native()
        .expect(format!("Failed to open port {}", &tty_string).as_str());
    port.set_exclusive(false).unwrap();

    let mut comm = serialcomunicator::SerialComunicator::<TTYPort, TTYPort>::new(port);
    comm.add("Dog", move |x| {
        x.write(b"ciao");
        FunctorRes::new()
    });
    let handler = comm.start();

    handler.join();
}
