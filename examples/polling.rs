extern crate mio;
extern crate mio_byte_fifo;

use std::io::{Read, Write, ErrorKind};
use std::thread;

use mio::{Poll, Events, Token, Ready, PollOpt};

use mio_byte_fifo::{Producer, Consumer};


fn main() {
    const FIFO_SIZE: usize = 16;
    const READ_BUF_SIZE: usize = 7;
    const EVENTS_CAPACITY: usize = 4;
    
    let (mut producer, mut consumer) = mio_byte_fifo::create(FIFO_SIZE);
    let message = "The quick brown fox jumps over the lazy dog";

    println!("sending message: '{}'", message);

    let producer_thread = thread::spawn(move || {
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(EVENTS_CAPACITY);
        let data = message.as_bytes();
        let mut pos = 0;

        let write_data_part = |producer: &mut Producer, pos: &mut usize| {
            loop {
                match producer.write(&data[*pos..]) {
                    Ok(n) => {
                        println!(
                            "sent     {} bytes: '{}'", n,
                            std::str::from_utf8(&data[*pos..(*pos + n)]).unwrap()
                        );
                        *pos += n;
                        if *pos >= data.len() {
                            break false;
                        }
                    },
                    Err(err) => match err.kind() {
                        ErrorKind::WouldBlock => break true,
                        _ => panic!("{:?}", err),
                    }
                }
            }
        };

        poll.register(&producer, Token(0), Ready::readable(), PollOpt::edge()).unwrap();
        
        if !write_data_part(&mut producer, &mut pos) {
            return;
        }

        'outer: loop {
            poll.poll(&mut events, None).unwrap();

            for event in events.iter() {
                assert_eq!(event.token(), Token(0));
                assert!(event.readiness().is_readable());
                if !write_data_part(&mut producer, &mut pos) {
                    break 'outer;
                }
            }
        }
    });

    let consumer_thread = thread::spawn(move || {
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(EVENTS_CAPACITY);
        let mut data = String::new();
        let mut buf = [0; READ_BUF_SIZE];

        let mut read_data_part = |consumer: &mut Consumer, data: &mut String| {
            loop {
                match consumer.read(&mut buf) {
                    Ok(n) => {
                        let str_part = std::str::from_utf8(&buf[0..n]).unwrap();
                        println!("received {} bytes: '{}'", n, str_part);
                        data.push_str(str_part);
                    },
                    Err(err) => {
                        match err.kind() {
                            ErrorKind::BrokenPipe => break false,
                            ErrorKind::WouldBlock => break true,
                            _ => panic!("{:?}", err),
                        }
                    }
                }
            }
        };

        poll.register(&consumer, Token(0), Ready::readable(), PollOpt::edge()).unwrap();
        
        'outer: loop {
            poll.poll(&mut events, None).unwrap();

            for event in events.iter() {
                assert_eq!(event.token(), Token(0));
                assert!(event.readiness().is_readable());
                if !read_data_part(&mut consumer, &mut data) {
                    break 'outer;
                }
            }
        }
        data
    });

    producer_thread.join().unwrap();
    let received_message = consumer_thread.join().unwrap();

    println!("received message: '{}'", received_message);
    assert_eq!(message, received_message);
}
