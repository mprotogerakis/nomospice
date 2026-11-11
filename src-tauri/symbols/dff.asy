Version 4
SymbolType CELL
RECTANGLE Normal -32 0 32 96
LINE Normal -48 16 -32 16
LINE Normal -48 80 -32 80
LINE Normal -32 72 -24 80
LINE Normal -24 80 -32 88
LINE Normal 32 16 48 16
CIRCLE Normal 32 76 40 84
LINE Normal 40 80 48 80
TEXT -26 16 Left 1 1D
TEXT -20 80 Left 1 C1
WINDOW 0 -32 -8 Left 2
WINDOW 3 -32 108 Left 2
SYMATTR Value dff
SYMATTR SpiceLine Vhigh=5 Vlow=0 Td=10n
SYMATTR Prefix X
SYMATTR Description D flip-flop, rising clock edge (logic levels Vlow/Vhigh, threshold halfway; Td: the outputs' time constant)
PIN -48 16 NONE 0
PINATTR PinName D
PINATTR SpiceOrder 1
PIN -48 80 NONE 0
PINATTR PinName C
PINATTR SpiceOrder 2
PIN 48 16 NONE 0
PINATTR PinName Q
PINATTR SpiceOrder 3
PIN 48 80 NONE 0
PINATTR PinName Qn
PINATTR SpiceOrder 4
