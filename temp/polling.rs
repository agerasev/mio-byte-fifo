extern crate mio;
extern crate mio_byte_fifo;

use std::io::{Read, Write};
use std::thread;

use mio::{Poll, Events, Token, Ready, PollOpt};


fn main() {
    let (mut p, mut c) = create(4);

    let pjh = thread::spawn(move || {
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);
        let data = "The quick brown fox jumps over the lazy dog".as_bytes();
        let mut pos = 0;

        poll.register(&c, Token(0), Ready::readable(), PollOpt::edge()).unwrap();
        
        loop {
            poll.poll(&mut events, None).unwrap();

            for events in events {
                if event.token() == Token(0) && event.readiness().is_readable() {
                    let n = p.write(data[pos..]).unwrap();
                    pos += n;
                    println!("sent");
                }
            }
        }
    });
        match c.read(&mut buf) {
            Ok(_) => panic!(),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::BrokenPipe);
                assert_eq!(err.get_ref().unwrap().description(), "Channel disconnected");
            }
        }

    let pjh = thread::spawn(move || {
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);

        assert_eq!(p.write(&[0; SIZE]).unwrap(), SIZE);
        poll.register(&p, Token(0), Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
        poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();

        let event = events.iter().next().unwrap();
        assert_eq!(event.token().0, 0);
        assert!(event.readiness().is_readable());
        assert!(!event.readiness().is_writable());
        assert_eq!(p.write(&[1; SIZE/2]).unwrap(), SIZE/2);
        poll.reregister(&p, Token(0), Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
        poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();

        thread::sleep(Duration::from_millis(10));
    });

    pjh.join().unwrap();
    cjh.join().unwrap();
}