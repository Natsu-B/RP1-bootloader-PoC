use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    crate::logging::puts("[PANIC] ");
    crate::logging::_print(format_args!("{info}\n"));
    crate::halt()
}
