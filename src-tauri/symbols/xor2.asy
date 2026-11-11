Version 4
SymbolType CELL
RECTANGLE Normal -32 0 32 64
LINE Normal -48 16 -32 16
LINE Normal -48 48 -32 48
LINE Normal 32 32 48 32
TEXT 0 18 Center 2 =1
WINDOW 0 -32 -8 Left 2
WINDOW 3 -32 76 Left 2
SYMATTR Value xor2
SYMATTR SpiceLine Vhigh=5 Vlow=0 Td=10n
SYMATTR Prefix X
SYMATTR Description XOR gate (logic levels Vlow/Vhigh, threshold halfway; Td: the output's time constant)
PIN -48 16 NONE 0
PINATTR PinName A
PINATTR SpiceOrder 1
PIN -48 48 NONE 0
PINATTR PinName B
PINATTR SpiceOrder 2
PIN 48 32 NONE 0
PINATTR PinName Y
PINATTR SpiceOrder 3
