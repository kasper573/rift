use std::cell::RefCell;

use rift::{App, Builder, ClientId, Server};

struct Ping;
struct Pong;
struct Spin;

thread_local! {
    static LOG: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

fn push(value: u32) {
    LOG.with_borrow_mut(|log| log.push(value));
}

fn taken() -> Vec<u32> {
    LOG.with_borrow_mut(std::mem::take)
}

#[test]
fn systems_run_in_registration_order() {
    fn one(b: &mut Builder) {
        b.system(|_| push(1));
    }
    fn two(b: &mut Builder) {
        b.system(|_| push(2));
    }
    let mut app = App::new(&[two, one]);
    let mut server = Server::new();
    app.start(&mut server);
    app.tick(&mut server, 0.1);
    assert_eq!(taken(), vec![2, 1]);
}

#[test]
fn events_dispatch_after_systems_and_cascade() {
    fn emit_ping(b: &mut Builder) {
        b.system(|ctx| ctx.events.emit(Ping));
    }
    fn on_ping(b: &mut Builder) {
        b.on::<Ping>(|ctx, _| {
            push(10);
            ctx.events.emit(Pong);
        });
    }
    fn on_pong(b: &mut Builder) {
        b.on::<Pong>(|_, _| push(20));
    }
    let mut app = App::new(&[emit_ping, on_ping, on_pong]);
    let mut server = Server::new();
    app.start(&mut server);
    app.tick(&mut server, 0.1);
    assert_eq!(taken(), vec![10, 20]);
}

#[test]
fn event_cascade_is_bounded() {
    fn spin(b: &mut Builder) {
        b.system(|ctx| ctx.events.emit(Spin));
        b.on::<Spin>(|ctx, _| {
            push(0);
            ctx.events.emit(Spin);
        });
    }
    let mut app = App::new(&[spin]);
    let mut server = Server::new();
    app.start(&mut server);
    app.tick(&mut server, 0.1);
    assert_eq!(taken().len(), 16);
}

#[test]
fn connect_hook_fires_on_connection() {
    fn greeter(b: &mut Builder) {
        b.connect(|_, _| push(99));
    }
    let mut app = App::new(&[greeter]);
    let mut server = Server::new();
    app.start(&mut server);
    server.connect(ClientId(1));
    app.tick(&mut server, 0.1);
    assert_eq!(taken(), vec![99]);
}
