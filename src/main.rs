#![feature(iter_advance_by)]
#![feature(vec_into_raw_parts)]

use std::{
    borrow::{Borrow, BorrowMut},
    cell::{RefCell, RefMut},
    collections::VecDeque,
    env,
    io::{Read, Write},
    os::linux::raw::stat,
    process::exit,
    sync::{
        atomic::{AtomicI32, AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

use base64::Engine;
use openssl::{cipher::Cipher, cipher_ctx::CipherCtx};
use serialport::TTYPort;

use crate::{actions::FunctorRes, serialcomunicator::SerialComunicator};

mod actions;
mod serialcomunicator;

static X: &[u8; 16] = b"Some Crypto Text";

struct AesBlock {
    recompute: bool,
    data: [u8; 16],
}

impl AesBlock {
    const fn new(data: [u8; 16]) -> Self {
        Self {
            recompute: true,
            data,
        }
    }
}

static CACHED_VAL: Mutex<AesBlock> = Mutex::new(AesBlock::new(*X));

fn compute_aes() -> [u8; 16] {
    let mut locked_cache = CACHED_VAL.lock().unwrap();
    if locked_cache.recompute {
        locked_cache.recompute = false;
        let cipher = Cipher::aes_128_ecb();
        let mut data = *X;
        let key: [u8; 16] = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ];

        for _ in 0..1500 {
            let mut ctx = CipherCtx::new().unwrap();
            ctx.encrypt_init(Some(cipher), Some(&key), None).unwrap();

            let mut ciphertext = vec![];
            ctx.cipher_update_vec(&data, &mut ciphertext).unwrap();
            ctx.cipher_final_vec(&mut ciphertext).unwrap();
            data.copy_from_slice(&ciphertext[0..16]);
        }
        locked_cache.data = data;
    }
    drop(locked_cache);

    CACHED_VAL.lock().unwrap().data
}

fn aes_reset<T>(_writer: &mut T) -> FunctorRes<T>
where
    T: Write + 'static + Send + Sync,
{
    let mut locked_cache = CACHED_VAL.lock().unwrap();
    locked_cache.recompute = true;
    FunctorRes::new()
}

fn aes_send<T>(writer: &mut T) -> FunctorRes<T>
where
    T: Write + 'static + Send + Sync,
{
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let mut i = COUNTER.fetch_add(1, Ordering::AcqRel);
    if i >= 16 {
        i = 0;
        COUNTER.store(1, Ordering::Release);
    }
    let data = compute_aes();
    writer.write_all(&data[i..i + 1]).unwrap();
    #[cfg(debug_assertions)]
    println!("Send D[{}]: {:?}", i, &data[i..i + 1]);
    FunctorRes::new()
}

fn main() {
    // Prints each argument on a separate line
    let mut args = env::args();

    let tty_string = args.nth(1).unwrap_or("/dev/ttyUSB0".to_string());
    let tty_baud = u32::from_str_radix(&args.next().unwrap_or("9200".into()), 10).unwrap();
    let mut port = serialport::new(&tty_string, tty_baud)
        .timeout(Duration::from_secs(60))
        .open_native()
        .expect(format!("Failed to open port {}", &tty_string).as_str());
    port.set_exclusive(false).unwrap();

    let mut comm = serialcomunicator::SerialComunicator::<TTYPort, TTYPort>::new(port);
    comm.add("R\n", move |writer| {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let mut i = COUNTER.fetch_add(1, Ordering::AcqRel);
        if i >= 16 {
            i = 0;
            COUNTER.store(1, Ordering::Release);
        }
        #[cfg(debug_assertions)]
        println!("Send X[{}]: {:?}", i, &X[i..i + 1]);
        writer.write(&X[i..i + 1]).expect("Non riesco a inviare :(");
        FunctorRes::new()
    });

    comm.add("D\n", aes_send);
    comm.add("Q\n", aes_send);
    comm.add("Y\n", aes_reset);

    let handler = comm.start();

    handler.send(("N\n".into(), Box::new(move |_x| exit(-1))));
    handler.join();
}

#[cfg(test)]
mod test {
    use std::io::stdout;

    use super::*;

    #[test]
    fn test_aes() {
        aes_send(&mut stdout());
    }
}
