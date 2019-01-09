//! Concurrent non-blocking byte FIFO buffer intended for use in [`Mio`] poll
//!
//! # Simple example
//!
//! ```rust
//! # extern crate mio_byte_fifo;
//! 
//! use std::io::{Read, Write};
//! 
//! # fn main() {
//! let (mut producer, mut consumer) = mio_byte_fifo::create(16);
//! 
//! let data = [0, 1, 254, 255];
//! let n = producer.write(&data).unwrap();
//! println!("written {} bytes: {:?}", n, data);
//! 
//! let mut buf = [0; 8];
//! let n = consumer.read(&mut buf).unwrap();
//! println!("read    {} bytes: {:?}", n, &buf[0..n]);
//! 
//! assert_eq!(data, buf[0..n]);
//! # }
//! ```
//!
//! # More complicated example
//!
//! ```rust
//! # extern crate mio;
//! # extern crate mio_byte_fifo;
//! 
//! use std::io::{Read, Write, ErrorKind};
//! use std::thread;
//! 
//! use mio::{Poll, Events, Token, Ready, PollOpt};
//! 
//! use mio_byte_fifo::{Producer, Consumer};
//! 
//! 
//! # fn main() {
//! const FIFO_SIZE: usize = 16;
//! const READ_BUF_SIZE: usize = 7;
//! const EVENTS_CAPACITY: usize = 4;
//! 
//! let (mut producer, mut consumer) = mio_byte_fifo::create(FIFO_SIZE);
//! let message = "The quick brown fox jumps over the lazy dog";
//! 
//! println!("sending message: '{}'", message);
//! 
//! let producer_thread = thread::spawn(move || {
//!     let poll = Poll::new().unwrap();
//!     let mut events = Events::with_capacity(EVENTS_CAPACITY);
//!     let data = message.as_bytes();
//!     let mut pos = 0;
//! 
//!     let write_data_part = |producer: &mut Producer, pos: &mut usize| {
//!         loop {
//!             match producer.write(&data[*pos..]) {
//!                 Ok(n) => {
//!                     println!(
//!                         "sent     {} bytes: '{}'", n,
//!                         std::str::from_utf8(&data[*pos..(*pos + n)]).unwrap()
//!                     );
//!                     *pos += n;
//!                     if *pos >= data.len() {
//!                         break false;
//!                     }
//!                 },
//!                 Err(err) => match err.kind() {
//!                     ErrorKind::WouldBlock => break true,
//!                     _ => panic!("{:?}", err),
//!                 }
//!             }
//!         }
//!     };
//! 
//!     poll.register(&producer, Token(0), Ready::writable(), PollOpt::edge()).unwrap();
//!     
//!     if !write_data_part(&mut producer, &mut pos) {
//!         return;
//!     }
//! 
//!     'outer: loop {
//!         poll.poll(&mut events, None).unwrap();
//! 
//!         for event in events.iter() {
//!             assert_eq!(event.token(), Token(0));
//!             assert!(event.readiness().is_writable());
//! 
//!             if !write_data_part(&mut producer, &mut pos) {
//!                 break 'outer;
//!             }
//!         }
//!     }
//! });
//! 
//! let consumer_thread = thread::spawn(move || {
//!     let poll = Poll::new().unwrap();
//!     let mut events = Events::with_capacity(EVENTS_CAPACITY);
//!     let mut data = String::new();
//!     let mut buf = [0; READ_BUF_SIZE];
//! 
//!     let mut read_data_part = |consumer: &mut Consumer, data: &mut String| {
//!         loop {
//!             match consumer.read(&mut buf) {
//!                 Ok(n) => {
//!                     let str_part = std::str::from_utf8(&buf[0..n]).unwrap();
//!                     println!("received {} bytes: '{}'", n, str_part);
//!                     data.push_str(str_part);
//!                 },
//!                 Err(err) => {
//!                     match err.kind() {
//!                         ErrorKind::BrokenPipe => break false,
//!                         ErrorKind::WouldBlock => break true,
//!                         _ => panic!("{:?}", err),
//!                     }
//!                 }
//!             }
//!         }
//!     };
//! 
//!     poll.register(&consumer, Token(0), Ready::readable(), PollOpt::edge()).unwrap();
//!     
//!     'outer: loop {
//!         poll.poll(&mut events, None).unwrap();
//! 
//!         for event in events.iter() {
//!             assert_eq!(event.token(), Token(0));
//!             assert!(event.readiness().is_readable());
//!             if !read_data_part(&mut consumer, &mut data) {
//!                 break 'outer;
//!             }
//!         }
//!     }
//!     data
//! });
//! 
//! producer_thread.join().unwrap();
//! let received_message = consumer_thread.join().unwrap();
//! 
//! println!("received message: '{}'", received_message);
//! assert_eq!(message, received_message);
//! # }
//! ```
//!
//! [`Mio`]: https://docs.rs/mio/
//!

extern crate mio;
extern crate rb;


use std::io::{Write, Read, Error, ErrorKind};
use std::sync::{Arc, atomic::{fence, AtomicBool, Ordering}};

use mio::{Evented, Poll, Token, Ready, PollOpt, Registration, SetReadiness};

use rb::{RB, RbError, RbProducer, RbConsumer, RbInspector};


pub struct Producer {
    reg: Registration,
    src: SetReadiness,
    rb:  Arc<rb::SpscRb<u8>>,
    rbp: rb::Producer<u8>,
    cls: Arc<AtomicBool>,
}

pub struct Consumer {
    reg: Registration,
    srp: SetReadiness,
    rb:  Arc<rb::SpscRb<u8>>,
    rbc: rb::Consumer<u8>,
    cls: Arc<AtomicBool>,
}

pub fn create(capacity: usize) -> (Producer, Consumer) {
    let flag = Arc::new(AtomicBool::new(true));

    let rb = Arc::new(rb::SpscRb::new(capacity));

    let (regp, srp) = Registration::new2();
    let (regc, src) = Registration::new2();

    let (rbp, rbc) = (rb.producer(), rb.consumer());

    let prod = Producer { reg: regp, src, rb: rb.clone(), rbp, cls: flag.clone() };
    let cons = Consumer { reg: regc, srp, rb, rbc, cls: flag };

    (prod, cons)
}

impl Evented for Producer {
    fn register(&self, poll: &Poll, token: Token, interest: Ready, poll_opt: PollOpt) -> Result<(), Error> {
        poll.register(&self.reg, token, interest, poll_opt)
    }

    fn reregister(&self, poll: &Poll, token: Token, interest: Ready, poll_opt: PollOpt) -> Result<(), Error> {
        poll.reregister(&self.reg, token, interest, poll_opt)
    }

    fn deregister(&self, poll: &Poll) -> Result<(), Error> {
        poll.deregister(&self.reg)
    }
}

impl Evented for Consumer {
    fn register(&self, poll: &Poll, token: Token, interest: Ready, poll_opt: PollOpt) -> Result<(), Error> {
        poll.register(&self.reg, token, interest, poll_opt)
    }

    fn reregister(&self, poll: &Poll, token: Token, interest: Ready, poll_opt: PollOpt) -> Result<(), Error> {
        poll.reregister(&self.reg, token, interest, poll_opt)
    }

    fn deregister(&self, poll: &Poll) -> Result<(), Error> {
        poll.deregister(&self.reg)
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        self.cls.store(false, Ordering::SeqCst);
        self.src.set_readiness(Ready::all()).unwrap();
        fence(Ordering::SeqCst);
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        self.cls.store(false, Ordering::SeqCst);
        self.srp.set_readiness(Ready::all()).unwrap();
        fence(Ordering::SeqCst);
    }
}

impl Write for Producer {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        if !self.cls.load(Ordering::SeqCst) {
            return Err(Error::new(
                ErrorKind::BrokenPipe,
                "Consumer was closed",
            ))
        }

        let empty = self.rb.is_empty();
        match self.rbp.write(buf) {
            Ok(num) => {
                if num > 0 && empty {
                    let res = self.src.set_readiness(Ready::readable());
                    fence(Ordering::SeqCst);
                    res
                } else {
                    Ok(())
                }.and(Ok(num))
            },
            Err(err) => match err {
                RbError::Full => Err(Error::new(
                    ErrorKind::WouldBlock,
                    "Ring buffer is full",
                )),
                RbError::Empty => unreachable!(),
            }
        }
    }

    fn flush(&mut self) -> Result<(), Error> {
        Ok(())
    }

}

impl Read for Consumer {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let full = self.rb.is_full();
        match self.rbc.read(buf) {
            Ok(num) => {
                if num > 0 && full {
                    let res = self.srp.set_readiness(Ready::writable());
                    fence(Ordering::SeqCst);
                    res
                } else {
                    Ok(())
                }.and(Ok(num))
            },
            Err(err) => match err {
                RbError::Empty => Err({
                    if !self.cls.load(Ordering::SeqCst) {
                        Error::new(
                            ErrorKind::BrokenPipe,
                            "Producer was closed",
                        )
                    } else {
                        Error::new(
                            ErrorKind::WouldBlock,
                            "Ring buffer is empty",
                        )
                    }
                }),
                RbError::Full => unreachable!(),
            }
        }
    }
}


#[cfg(test)]
mod test {
    use super::*;

    use std::thread;
    use std::time::{Duration};

    use mio::{Events};


    #[test]
    fn reg_set_r() {
        let (reg, sr) = Registration::new2();
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);

        let jh = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            sr.set_readiness(Ready::readable()).unwrap();
        });

        poll.register(&reg, Token(0), Ready::readable(), PollOpt::edge()).unwrap();

        poll.poll(&mut events, Some(Duration::from_secs(1))).unwrap();
        let mut hdl = false;
        for e in events.iter() {
            assert_eq!(e.token().0, 0);
            assert!(e.readiness().is_readable());
            hdl = true;
        }
        assert!(hdl);

        poll.deregister(&reg).unwrap();

        jh.join().unwrap();
    }

    #[test]
    fn reg_set_w() {
        let (reg, sr) = Registration::new2();
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);

        let jh = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            sr.set_readiness(Ready::writable()).unwrap();
        });

        poll.register(&reg, Token(0), Ready::writable(), PollOpt::edge()).unwrap();

        poll.poll(&mut events, Some(Duration::from_secs(1))).unwrap();
        let mut hdl = false;
        for e in events.iter() {
            assert_eq!(e.token().0, 0);
            assert!(e.readiness().is_writable());
            hdl = true;
        }
        assert!(hdl);

        poll.deregister(&reg).unwrap();

        jh.join().unwrap();
    }

    #[test]
    fn reg_set_twice() {
        let (reg, sr) = Registration::new2();
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);

        let jh = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            sr.set_readiness(Ready::readable()).unwrap();

            thread::sleep(Duration::from_millis(10));
            sr.set_readiness(Ready::readable()).unwrap();
        });

        poll.register(&reg, Token(0), Ready::readable(), PollOpt::edge()).unwrap();

        poll.poll(&mut events, Some(Duration::from_secs(1))).unwrap();
        let mut hdl = false;
        for e in events.iter() {
            assert_eq!(e.token().0, 0);
            assert!(e.readiness().is_readable());
            hdl = true;
        }
        assert!(hdl);

        poll.poll(&mut events, Some(Duration::from_secs(1))).unwrap();
        let mut hdl = false;
        for e in events.iter() {
            assert_eq!(e.token().0, 0);
            assert!(e.readiness().is_readable());
            hdl = true;
        }
        assert!(hdl);

        poll.deregister(&reg).unwrap();

        jh.join().unwrap();
    }

    #[test]
    fn reg_drop() {
        let (reg, sr) = Registration::new2();

        let jh = thread::spawn(move || {
            let _ = reg;
        });

        thread::sleep(Duration::from_millis(10));
        sr.set_readiness(Ready::readable()).unwrap();

        jh.join().unwrap();
    }

    #[test]
    fn dereg_reg() {
        let (reg, sr) = Registration::new2();
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);

        let jh = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            sr.set_readiness(Ready::readable()).unwrap();

            thread::sleep(Duration::from_millis(10));
            sr.set_readiness(Ready::readable()).unwrap();
        });

        poll.register(&reg, Token(0), Ready::readable(), PollOpt::edge()).unwrap();

        poll.poll(&mut events, Some(Duration::from_secs(1))).unwrap();
        let mut hdl = false;
        for e in events.iter() {
            assert_eq!(e.token().0, 0);
            assert!(e.readiness().is_readable());
            hdl = true;
        }
        assert!(hdl);

        poll.deregister(&reg).unwrap();

        poll.register(&reg, Token(1), Ready::readable(), PollOpt::edge()).unwrap();

        poll.poll(&mut events, Some(Duration::from_secs(1))).unwrap();
        let mut hdl = false;
        for e in events.iter() {
            assert_eq!(e.token().0, 1);
            assert!(e.readiness().is_readable());
            hdl = true;
        }
        assert!(hdl);

        poll.deregister(&reg).unwrap();

        jh.join().unwrap();
    }
    
    #[test]
    fn write_read() {
        let (mut p, mut c) = create(16);

        assert_eq!(p.write(b"abcdef").unwrap(), 6);

        let mut buf = [0; 6];
        assert_eq!(c.read(&mut buf).unwrap(), 6);
        assert_eq!(&buf, b"abcdef");
    }

    #[test]
    fn write_read_concat() {
        let (mut p, mut c) = create(16);

        assert_eq!(p.write(b"abc").unwrap(), 3);
        assert_eq!(p.write(b"def").unwrap(), 3);

        let mut buf = [0; 6];
        assert_eq!(c.read(&mut buf).unwrap(), 6);
        assert_eq!(&buf, b"abcdef");
    }

    #[test]
    fn write_read_split() {
        let (mut p, mut c) = create(16);

        assert_eq!(p.write(b"abcdef").unwrap(), 6);

        let mut buf = [0; 3];
        assert_eq!(c.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, b"abc");
        assert_eq!(c.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, b"def");
    }

    #[test]
    fn write_read_empty() {
        let (mut p, mut c) = create(16);

        let mut buf = [0; 6];
        
        assert_eq!(p.write(b"abc").unwrap(), 3);
        assert_eq!(c.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, b"abc\0\0\0");

        assert_eq!(p.write(b"def").unwrap(), 3);
        assert_eq!(c.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, b"def\0\0\0");
    }

    #[test]
    fn write_read_full() {
        let (mut p, mut c) = create(8);

        let range: Vec<u8> = (0..8).collect();
        let mut buf = [0; 6];
        
        assert_eq!(p.write(&range).unwrap(), 8);

        assert_eq!(c.read(&mut buf[0..3]).unwrap(), 3);
        assert_eq!(&buf[0..3], &[0,1,2]);

        assert_eq!(p.write(b"abcdef").unwrap(), 3);

        assert_eq!(c.read(&mut buf[0..3]).unwrap(), 3);
        assert_eq!(&buf[0..3], &[3,4,5]);

        assert_eq!(c.read(&mut buf).unwrap(), 5);
        assert_eq!(&buf[0..5], &[6,7,b'a',b'b',b'c']);
    }

    #[test]
    fn read_block() {
        let (_p, mut c) = create(16);

        let mut buf = [0; 4];
        match c.read(&mut buf) {
            Ok(_) => panic!(),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::WouldBlock);
                assert_eq!(err.get_ref().unwrap().description(), "Ring buffer is empty");
            }
        }
    }

    #[test]
    fn write_block() {
        const SIZE: usize = 16;
        let (mut p, _c) = create(SIZE);

        assert_eq!(p.write(&[0; SIZE]).unwrap(), SIZE);
        match p.write(b"abc") {
            Ok(_) => panic!(),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::WouldBlock);
                assert_eq!(err.get_ref().unwrap().description(), "Ring buffer is full");
            }
        }
    }

    #[test]
    fn close_cons() {
        let (mut p, c) = create(16);

        assert_eq!(p.write(b"abc").unwrap(), 3);

        (move || {
            let _ = c;
        })();

        match p.write(b"def") {
            Ok(_) => panic!(),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::BrokenPipe);
                assert_eq!(err.get_ref().unwrap().description(), "Consumer was closed");
            }
        }
    }

    #[test]
    fn close_prod() {
        let (mut p, mut c) = create(16);
        let mut buf = [0; 6];

        assert_eq!(p.write(b"abcdef").unwrap(), 6);

        assert_eq!(c.read(&mut buf[0..3]).unwrap(), 3);
        assert_eq!(&buf[0..3], b"abc");

        (move || {
            let _ = p;
        })();

        assert_eq!(c.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf[0..3], b"def");

        match c.read(&mut buf) {
            Ok(_) => panic!(),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::BrokenPipe);
                assert_eq!(err.get_ref().unwrap().description(), "Producer was closed");
            }
        }
    }

    #[test]
    fn poll_cons() {
        let (mut p, mut c) = create(16);
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);
        let mut buf = [0; 6];

        poll.register(&c, Token(0), Ready::readable(), PollOpt::edge()).unwrap();

        let jh = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            assert_eq!(p.write(b"abc").unwrap(), 3);
            assert_eq!(p.write(b"def").unwrap(), 3);
            p
        });

        poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();
        thread::sleep(Duration::from_millis(10));

        {
            let mut eiter = events.iter();

            let event = eiter.next().unwrap();
            assert_eq!(event.token().0, 0);
            assert!(event.readiness().is_readable());
            assert_eq!(c.read(&mut buf).unwrap(), 6);
            assert_eq!(&buf, b"abcdef");

            assert!(eiter.next().is_none());
        }

        poll.poll(&mut events, Some(Duration::from_millis(10))).unwrap();
        assert!(events.iter().next().is_none());

        jh.join().unwrap();
    }

    #[test]
    fn poll_prod() {
        const SIZE: usize = 16;
        let (mut p, mut c) = create(SIZE);
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);

        poll.register(&p, Token(0), Ready::writable(), PollOpt::edge()).unwrap();

        assert_eq!(p.write(&[0; SIZE]).unwrap(), SIZE);

        let jh = thread::spawn(move || {
            let mut buf = [0; 3];
            thread::sleep(Duration::from_millis(10));
            assert_eq!(c.read(&mut buf).unwrap(), 3);
            assert_eq!(c.read(&mut buf).unwrap(), 3);
            c
        });

        poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();
        thread::sleep(Duration::from_millis(10));

        {
            let mut eiter = events.iter();

            let event = eiter.next().unwrap();
            assert_eq!(event.token().0, 0);

            assert!(event.readiness().is_writable());

            assert_eq!(p.write(b"abcdefghi").unwrap(), 6);

            assert!(eiter.next().is_none());
        }

        poll.poll(&mut events, Some(Duration::from_millis(10))).unwrap();
        assert!(events.iter().next().is_none());

        jh.join().unwrap();
    }

    #[test]
    fn poll_cons_close() {
        let (p, mut c) = create(16);
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);
        let mut buf = [0; 3];

        poll.register(&c, Token(0), Ready::readable(), PollOpt::edge()).unwrap();

        let jh = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            let _ = p;
        });

        'outer: for _ in 0..2 {
            poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();

            for event in events.iter() {
                assert_eq!(event.token().0, 0);
                assert!(event.readiness().is_readable());
                match c.read(&mut buf) {
                    Ok(_) => panic!(),
                    Err(err) => {
                        match err.kind() {
                            ErrorKind::BrokenPipe => break 'outer,
                            ErrorKind::WouldBlock => (),
                            _ => panic!("{:?}", err),
                        }
                    }
                }
            }
        }

        jh.join().unwrap();
    }

    #[test]
    fn poll_prod_close() {
        const SIZE: usize = 16;
        let (mut p, c) = create(SIZE);
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);

        poll.register(&p, Token(0), Ready::writable(), PollOpt::edge()).unwrap();

        assert_eq!(p.write(&[0; SIZE]).unwrap(), SIZE);

        let jh = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            let _ = c;
        });

        'outer: loop {
            poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();

            for event in events.iter() {
                assert_eq!(event.token().0, 0);
                assert!(event.readiness().is_writable());
                match p.write(b"def") {
                    Ok(_) => panic!(),
                    Err(err) => {
                        match err.kind() {
                            ErrorKind::BrokenPipe => break 'outer,
                            ErrorKind::WouldBlock => (),
                            _ => panic!("{:?}", err),
                        }
                    }
                }
            }
        }

        jh.join().unwrap();
    }

    #[test]
    fn poll_prod_cons() {
        const SIZE: usize = 16;
        let (mut p, mut c) = create(SIZE);

        let cjh = thread::spawn(move || {
            let poll = Poll::new().unwrap();
            let mut events = Events::with_capacity(16);
            let mut buf = [0; SIZE/2];

            poll.register(&c, Token(0), Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
            
            let mut i = 0;
            'outer: loop {
                poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();
                for event in events.iter() {
                    assert_eq!(event.token().0, 0);
                    assert!(event.readiness().is_readable());
                    'inner: loop {
                        match c.read(&mut buf) {
                            Ok(n) => {
                                assert_eq!(n, SIZE/2);
                                assert_eq!(&buf, &[i/2; SIZE/2]);
                                i += 1;
                            },
                            Err(err) => {
                                match err.kind() {
                                    ErrorKind::BrokenPipe => break 'outer,
                                    ErrorKind::WouldBlock => break 'inner,
                                    _ => panic!("{:?}", err),
                                }
                            }
                        }
                    }
                }
                poll.reregister(&c, Token(0), Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
            }
            assert_eq!(i, 3);
        });

        let pjh = thread::spawn(move || {
            let poll = Poll::new().unwrap();
            let mut events = Events::with_capacity(16);

            assert_eq!(p.write(&[0; SIZE]).unwrap(), SIZE);
            poll.register(&p, Token(0), Ready::writable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
            poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();

            let event = events.iter().next().unwrap();
            assert_eq!(event.token().0, 0);
            assert!(event.readiness().is_writable());
            assert_eq!(p.write(&[1; SIZE/2]).unwrap(), SIZE/2);
            poll.reregister(&p, Token(0), Ready::writable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
            poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();

            thread::sleep(Duration::from_millis(10));
        });

        pjh.join().unwrap();
        cjh.join().unwrap();
    }
}
