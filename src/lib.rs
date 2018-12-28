//! Concurrent non-blocking byte SPSC FIFO buffer intended for use in [`Mio`] poll


extern crate mio;
extern crate mio_extras;
extern crate rb;


use std::io::{Write, Read, Error, ErrorKind};
use std::sync::mpsc::{TryRecvError};

use mio::{Evented, Poll, Token, Ready, PollOpt};
use mio_extras::channel::{channel, Sender, Receiver, SendError};

use rb::{RB, RbError, RbProducer, RbConsumer};


pub struct Handle<T> {
    tx: Sender<()>,
    rx: Receiver<()>,
    rb: T,
}

pub type Producer = Handle<rb::Producer<u8>>;
pub type Consumer = Handle<rb::Consumer<u8>>;

pub fn create(capacity: usize) -> (Producer, Consumer) {
    let rb = rb::SpscRb::new(capacity);
    let (rbp, rbc) = (rb.producer(), rb.consumer());

    let (txp, rxc) = channel();
    let (txc, rxp) = channel();

    (Producer { tx: txp, rx: rxp, rb: rbp }, Consumer { tx: txc, rx: rxc, rb: rbc })
}


impl<T> Evented for Handle<T> {
    fn register(&self, poll: &Poll, token: Token, interest: Ready, poll_opt: PollOpt) -> Result<(), Error> {
        poll.register(&self.rx, token, interest, poll_opt)
    }
    fn reregister(&self, poll: &Poll, token: Token, interest: Ready, poll_opt: PollOpt) -> Result<(), Error> {
        poll.reregister(&self.rx, token, interest, poll_opt)
    }
    fn deregister(&self, poll: &Poll) -> Result<(), Error> {
        poll.deregister(&self.rx)
    }
}

impl<T> Handle<T> {
    fn drain(&mut self) -> Result<(), Error> {
        loop {
            match self.rx.try_recv() {
                Ok(()) => continue,
                Err(err) => match err {
                    TryRecvError::Empty => break Ok(()),
                    TryRecvError::Disconnected => break Err(Error::new(
                        ErrorKind::BrokenPipe,
                        "Channel disconnected",
                    )),
                }
            }
        }
    }

    fn notify(&mut self) -> Result<(), Error> {
        match self.tx.send(()) {
            Ok(()) => Ok(()),
            Err(err) => match err {
                SendError::Io(e) => Err(e),
                SendError::Disconnected(()) => Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "Channel disconnected",
                )),
            }
        }
    }
}

impl<T> Drop for Handle<T> {
    fn drop(&mut self) {
        let _ = self.notify();
    }
}

impl Write for Producer {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        self.drain()?;
        match self.rb.write(buf) {
            Ok(num) => {
                if num > 0 {
                    self.notify()?;
                }
                Ok(num)
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
        let dres = self.drain();
        match self.rb.read(buf) {
            Ok(num) => {
                if num > 0 && dres.is_ok() {
                    self.notify()?;
                }
                Ok(num)
            },
            Err(err) => match err {
                RbError::Empty => {
                    match dres {
                        Ok(()) => Err(Error::new(
                            ErrorKind::WouldBlock,
                            "Ring buffer is empty",
                        )),
                        Err(e) => Err(e),
                    }
                },
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
                assert_eq!(err.get_ref().unwrap().description(), "Channel disconnected");
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
                assert_eq!(err.get_ref().unwrap().description(), "Channel disconnected");
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
            assert!(!event.readiness().is_writable());
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

        // We should register producer as `readable`
        // because poll mechanism if based on underlying channels
        poll.register(&p, Token(0), Ready::readable(), PollOpt::edge()).unwrap();

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

            // Thats all right, we can write data when producer is `readable`
            assert!(event.readiness().is_readable());
            assert!(!event.readiness().is_writable());

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

        poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();
        
        let mut eiter = events.iter();

        let event = eiter.next().unwrap();
        assert_eq!(event.token().0, 0);
        assert!(event.readiness().is_readable());
        assert!(!event.readiness().is_writable());
        match c.read(&mut buf) {
            Ok(_) => panic!(),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::BrokenPipe);
                assert_eq!(err.get_ref().unwrap().description(), "Channel disconnected");
            }
        }

        assert!(eiter.next().is_none());

        jh.join().unwrap();
    }

    #[test]
    fn poll_prod_close() {
        const SIZE: usize = 16;
        let (mut p, c) = create(SIZE);
        let poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(16);

        poll.register(&p, Token(0), Ready::readable(), PollOpt::edge()).unwrap();

        assert_eq!(p.write(&[0; SIZE]).unwrap(), SIZE);

        let jh = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            let _ = c;
        });

        poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();

        let mut eiter = events.iter();

        let event = eiter.next().unwrap();
        assert_eq!(event.token().0, 0);

        assert!(event.readiness().is_readable());
        assert!(!event.readiness().is_writable());

        match p.write(b"def") {
            Ok(_) => panic!(),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::BrokenPipe);
                assert_eq!(err.get_ref().unwrap().description(), "Channel disconnected");
            }
        }

        assert!(eiter.next().is_none());

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
            poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();
            
            for i in 0..3 {
                let event = events.iter().next().unwrap();
                assert_eq!(event.token().0, 0);
                assert!(event.readiness().is_readable());
                assert!(!event.readiness().is_writable());
                assert_eq!(c.read(&mut buf).unwrap(), SIZE/2);
                assert_eq!(&buf, &[i/2; SIZE/2]);
                poll.reregister(&c, Token(0), Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
                poll.poll(&mut events, Some(Duration::from_secs(10))).unwrap();
            }

            let event = events.iter().next().unwrap();
            assert_eq!(event.token().0, 0);
            assert!(event.readiness().is_readable());
            assert!(!event.readiness().is_writable());
            match c.read(&mut buf) {
                Ok(_) => panic!(),
                Err(err) => {
                    assert_eq!(err.kind(), ErrorKind::BrokenPipe);
                    assert_eq!(err.get_ref().unwrap().description(), "Channel disconnected");
                }
            }
        });

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
}

