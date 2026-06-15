use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    crate::uart::puts("[PANIC] ");
    crate::uart::_print(format_args!("{info}\n"));
    crate::halt()
}
