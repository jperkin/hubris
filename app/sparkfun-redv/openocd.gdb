target extended-remote :3333

# print demangled symbols
set confirm off
set print asm-demangle on

# set backtrace limit to not have infinite backtrace loops
set backtrace limit 32

monitor arm semihosting enable

# program file into the flash
load

# start the process but immediately halt the processor
stepi
