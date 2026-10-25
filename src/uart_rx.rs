//! Fixed receive queue for the USART RX interrupt. No parsing in the ISR.

// One slot is reserved to distinguish full from empty: 31 usable bytes.
const SIZE: usize = 32;

struct Receiver {
    bytes: [u8; SIZE],
    head: u8,
    tail: u8,
    discarding: bool,
    error_pending: bool,
}

impl Receiver {
    const fn new() -> Self {
        Self {
            bytes: [0; SIZE],
            head: 0,
            tail: 0,
            discarding: false,
            error_pending: false,
        }
    }

    fn push(&mut self, byte: u8, damaged: bool) {
        if damaged {
            self.head = 0;
            self.tail = 0;
            self.discarding = true;
            // An erroneous CR/LF is not a trustworthy boundary.
            return;
        }

        if !self.discarding {
            let next = (self.head + 1) & (SIZE as u8 - 1);
            if next != self.tail {
                self.bytes[(self.head & (SIZE as u8 - 1)) as usize] = byte;
                self.head = next;
                return;
            }
            // Conservatively drop queued data, including any earlier full lines.
            // The main loop's partial command is invalidated by the error event.
            self.head = 0;
            self.tail = 0;
            self.discarding = true;
        }

        if byte == b'\r' || byte == b'\n' {
            self.discarding = false;
            self.error_pending = true;
        }
    }

    fn pop(&mut self) -> Option<Result<u8, ()>> {
        // Always precede bytes of the next line with an error notification.
        if self.error_pending {
            self.error_pending = false;
            return Some(Err(()));
        }
        if self.head == self.tail {
            return None;
        }
        let byte = self.bytes[(self.tail & (SIZE as u8 - 1)) as usize];
        self.tail = (self.tail + 1) & (SIZE as u8 - 1);
        Some(Ok(byte))
    }
}

#[cfg(not(test))]
use core::cell::UnsafeCell;

#[cfg(not(test))]
static RECEIVER: avr_device::interrupt::Mutex<UnsafeCell<Receiver>> =
    avr_device::interrupt::Mutex::new(UnsafeCell::new(Receiver::new()));

#[cfg(not(test))]
pub fn init() {
    // HAL has already initialized baud, framing and RX/TX. Enable only RX IRQ.
    let usart = unsafe { &*arduino_hal::pac::USART0::ptr() };
    usart.ucsr0b().modify(|_, w| w.rxcie0().set_bit());
}

#[cfg(not(test))]
pub fn pop() -> Option<Result<u8, ()>> {
    avr_device::interrupt::free(|cs| {
        // All mutable accesses are inside interrupt-free sections; no reference
        // escapes. The RX ISR cannot overlap this main-loop access.
        unsafe { &mut *RECEIVER.borrow(cs).get() }.pop()
    })
}

#[cfg(not(test))]
#[avr_device::interrupt(atmega328p)]
fn USART_RX() {
    // ATmega328P data-space addresses. Capture flags before popping the FIFO.
    // This ISR is the only reader of UDR0; Timer0 never touches these registers.
    let status = unsafe { core::ptr::read_volatile(0xc0 as *const u8) };
    let byte = unsafe { core::ptr::read_volatile(0xc6 as *const u8) };
    avr_device::interrupt::free(|cs| {
        // Interrupts are already disabled on ISR entry and remain disabled here.
        unsafe { &mut *RECEIVER.borrow(cs).get() }.push(byte, status & 0x1c != 0);
    });
}

#[cfg(test)]
#[path = "../tests/uart_rx.rs"]
mod tests;
