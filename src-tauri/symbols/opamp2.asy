Version 4
SymbolType CELL
LINE Normal -32 32 32 64
LINE Normal 32 64 -32 96
LINE Normal -32 96 -32 32
LINE Normal 0 32 0 48
LINE Normal 0 80 0 96
LINE Normal -26 48 -18 48
LINE Normal -26 80 -18 80
LINE Normal -22 76 -22 84
LINE Normal 4 38 10 38
LINE Normal 7 35 7 41
LINE Normal 4 90 10 90
WINDOW 0 16 32 Left 2
WINDOW 3 16 104 Left 2
SYMATTR Value opamp2
SYMATTR SpiceLine Aol=100K GBW=10Meg Slew=10Meg Vsat=0.1
SYMATTR Prefix X
SYMATTR Description Opamp with supply pins: output limited to the supplies less Vsat, slew rate (V/s); as Value also any opamp of the libraries (TL072 …)
PIN -32 80 NONE 0
PINATTR PinName In+
PINATTR SpiceOrder 1
PIN -32 48 NONE 0
PINATTR PinName In-
PINATTR SpiceOrder 2
PIN 0 32 NONE 0
PINATTR PinName V+
PINATTR SpiceOrder 3
PIN 0 96 NONE 0
PINATTR PinName V-
PINATTR SpiceOrder 4
PIN 32 64 NONE 0
PINATTR PinName OUT
PINATTR SpiceOrder 5
