use core::num;
use std::{
    collections::VecDeque,
    io::{BufRead, Read, Write},
    ops::{Deref, DerefMut},
    string,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use base64::Engine;
use serialport::TTYPort;

use crate::actions::{Actions, Functor};

enum BuffType {
    ASCII(usize),
    BYTES(usize),
}

pub struct SerialComunicator<TtyOut, TtyIn>
where
    TtyOut: Write + 'static + Send,
    TtyIn: Read,
{
    tty_recv: TtyIn,
    kill_signal: AtomicBool,
    handlers: Vec<JoinHandle<()>>,
    actions: Actions<TtyOut>,
}

impl<TtyOut, TtyIn> Deref for SerialComunicator<TtyOut, TtyIn>
where
    TtyOut: Write + 'static + Send,
    TtyIn: Read,
{
    type Target = Actions<TtyOut>;

    fn deref(&self) -> &Self::Target {
        &self.actions
    }
}

impl<TtyOut, TtyIn> DerefMut for SerialComunicator<TtyOut, TtyIn>
where
    TtyOut: Write + 'static + Send,
    TtyIn: Read,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.actions
    }
}

impl<T, X> SerialComunicator<T, X>
where
    T: Write + 'static + Send + Sync,
    X: Read + 'static + Send,
{
    pub fn new(port: TTYPort) -> SerialComunicator<TTYPort, TTYPort> {
        let tty_send = port.try_clone_native().expect("Molto triste");
        let tty_recv = port;

        SerialComunicator {
            tty_recv,
            kill_signal: AtomicBool::new(false),
            handlers: Vec::new(),
            actions: Actions::new(tty_send),
        }
    }

    pub fn start(self: Self) -> ComunicatorHandler<(String, Functor<T>)> {
        let Self {
            tty_recv,
            actions,
            mut handlers,
            kill_signal,
            ..
        } = self;

        let kill_share = Arc::new(kill_signal);

        let (buff_sender, buff_reciver) = mpsc::channel();
        let (func_sender, func_reciver) = mpsc::channel();
        let handler = thread::spawn({
            let kill_signal = kill_share.clone();
            move || {
                reciver_fn(kill_signal, tty_recv, buff_sender);
            }
        });

        handlers.push(handler);

        let handler = thread::spawn({
            let kill_signal = kill_share.clone();
            move || process_fn(kill_signal, actions, buff_reciver, func_reciver)
        });

        handlers.push(handler);
        ComunicatorHandler {
            func_sender,
            kill_thread: kill_share.clone(),
            handlers,
        }
    }
}

pub struct ComunicatorHandler<T> {
    func_sender: mpsc::Sender<T>,
    kill_thread: Arc<AtomicBool>,
    handlers: Vec<JoinHandle<()>>,
}

impl<T> Deref for ComunicatorHandler<T> {
    type Target = mpsc::Sender<T>;
    fn deref(&self) -> &Self::Target {
        &self.func_sender
    }
}

impl<T> DerefMut for ComunicatorHandler<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.func_sender
    }
}

impl<T> ComunicatorHandler<T> {
    pub fn kill(self: Self) {
        self.kill_thread.store(true, Ordering::Release);
    }

    pub fn join(self) {
        for hand in self.handlers {
            hand.join().expect("expected");
        }
    }
}

fn process_fn<U>(
    kill_signal: Arc<AtomicBool>,
    mut actions: Actions<U>,
    reciver: mpsc::Receiver<([u8; 400], BuffType)>,
    func_reciver: mpsc::Receiver<(String, Functor<U>)>,
) -> ()
where
    U: Write + 'static + Send + Sync,
{
    let mut dequeue = VecDeque::new();
    let mut lines = VecDeque::new();
    let mut ascii_mode = true;

    while !kill_signal.load(Ordering::Acquire) {
        while let Ok((key, func)) = func_reciver.recv_timeout(Duration::from_millis(200)) {
            actions.add_box(key, func);
        }

        match reciver.recv() {
            Ok((buff, BuffType::ASCII(n))) => {
                let _ = dequeue.write(&buff[0..n]);
            }
            Ok((buff, BuffType::BYTES(n))) => {
                let _ = dequeue.write(&buff[0..n]);
                ascii_mode = false;
            }

            Err(_) => eprintln!("Failed To Recive Data"),
        }

        if ascii_mode {
            actions = process_ascii_lines(&mut dequeue, &mut lines, actions);
        } else {
            actions = process_not_ascii_lines(&mut dequeue, &mut lines, &mut ascii_mode, actions);
        }
    }
}

fn process_not_ascii_lines<U>(
    dequeue: &mut VecDeque<u8>,
    lines: &mut VecDeque<String>,
    ascii_mode: &mut bool,
    mut actions: Actions<U>,
) -> Actions<U>
where
    U: Write + 'static + Send + Sync,
{
    let base64engine = base64::engine::general_purpose::STANDARD;
    dequeue.make_contiguous();
    let (slice, _) = dequeue.as_slices();
    let mut pairs = Vec::new();
    const WINDOWS_SIZE: usize = 4;

    let mut start = None;
    for i in 0..slice.len() - WINDOWS_SIZE {
        if &slice[i..i + WINDOWS_SIZE] == b"\xff\xfe\xfc\xfb" {
            match start {
                None => start = Some(i),
                Some(x) => {
                    pairs.push((x, i));
                    start = None;
                }
            }
        }
    }

    let mut alread_read = 0;
    for (start, end) in pairs {
        let start = start - alread_read;
        let end = end - alread_read;

        let mut string_builder = String::with_capacity(start);
        for _ in 0..start {
            let c = dequeue.pop_front().expect("Non puo accadere");
            match c {
                b'\n' => {
                    lines.push_back(string_builder);
                    string_builder = String::new();
                }
                b'\0' => {}
                c => string_builder.push(c.into()),
            }
        }

        alread_read += start;
        //Discard marshal
        dequeue.drain(0..WINDOWS_SIZE);
        let num_elem = end - start - 4;
        let mut tmp = vec![0; num_elem];
        dequeue.read_exact(&mut tmp[0..num_elem]).unwrap();
        let mut string = base64engine.encode(tmp);
        dequeue.drain(0..WINDOWS_SIZE);
        alread_read += 4 + end - start;
        string.push('\n');
        lines.push_back(string);
    }

    if let None = start {
        if !dequeue.contains(&b'\xff') {
            *ascii_mode = true;
            return process_ascii_lines(dequeue, lines, actions);
        }
    }

    for line in lines.drain(..) {
        print!("{}", line);
        actions = actions.apply(&line);
    }
    actions
}

fn process_ascii_lines<U>(
    dequeue: &mut VecDeque<u8>,
    lines: &mut VecDeque<String>,
    mut actions: Actions<U>,
) -> Actions<U>
where
    U: Write + 'static + Send + Sync,
{
    // while dequeue.contains(&b'\n') {
    while !dequeue.is_empty() {
        let mut tmp_s = String::new();
        tmp_s.push('a');
        while let Some(b'\x00') = dequeue.front() {
            dequeue.pop_front();
        }

        unsafe {
            if let Ok(_) = dequeue.read_exact(&mut tmp_s.as_bytes_mut()) {
                tmp_s.push('\n');
                lines.push_back(tmp_s);
            } else {
                break;
            }
        }
    }

    for line in lines.drain(..) {
        print!("{}", line);
        actions = actions.apply(&line);
    }
    actions
}

fn reciver_fn(
    kill_signal: Arc<AtomicBool>,
    mut port: impl Read,
    sender: mpsc::Sender<([u8; 400], BuffType)>,
) {
    let mut buff = [0; 400];
    while !kill_signal.load(Ordering::Acquire) {
        match port.read(&mut buff) {
            Ok(n) => {
                #[cfg(debug_assertions)]
                {
                    eprintln!("DEBUG: {:?}", &buff[0..n]);
                }
                if let Ok(s) = std::str::from_utf8(&buff[0..n]) {
                    if s.is_ascii() {
                        sender
                            .send((buff, BuffType::ASCII(n)))
                            .expect("Sending to process fn");
                    }
                } else {
                    sender
                        .send((buff, BuffType::BYTES(n)))
                        .expect("Sending to process fn");
                }
            }
            Err(x) => {
                eprintln!("Error TTY {:?}", x);
            }
        }
    }
}
