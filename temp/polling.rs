extern crate mio;
extern crate mio_byte_fifo;

use std::io::{Read, Write, ErrorKind};
use std::thread;

use mio::{Poll, Events, Token, Ready, PollOpt};


fn main() {
    let (mut p, mut c) = mio_byte_fifo::create(4);

    let pjh = thread::spawn(move || {
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);
        let data = "The quick brown fox jumps over the lazy dog".as_bytes();
        let mut pos = 0;

        poll.register(&p, Token(0), Ready::readable(), PollOpt::edge()).unwrap();
        
        'outer: loop {
            let n = p.write(&data[pos..]).unwrap();
            pos += n;
            println!("sent {} bytes", n);
            if n >= data.len() {
                break;
            }

            poll.poll(&mut events, None).unwrap();
        }
    });

    let cjh = thread::spawn(move || {
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);
        let mut data = Vec::new();
        let mut buf = [0; 4];

        poll.register(&c, Token(0), Ready::readable(), PollOpt::edge()).unwrap();
        
        'outer: loop {
            poll.poll(&mut events, None).unwrap();

            for _event in events.iter() {
                match c.read(&mut buf) {
                    Ok(n) => {
                        println!("recieved {} bytes", n);
                        data.extend_from_slice(&buf[0..n]);
                    },
                    Err(err) => {
                        if err.kind() == ErrorKind::BrokenPipe {
                            break 'outer;
                        } else {
                            panic!();
                        }
                    }
                };
                println!("sent");
            }
        }
    });

    pjh.join().unwrap();
    cjh.join().unwrap();
}