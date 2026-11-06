# Third-party notices

## OpenPCB (AGPL-3.0-or-later)

Design tokens (src/index.css), theme handling (src/lib/theme.ts, src/lib/utils.ts) and UI primitives
(src/ui/*) are taken from OpenPCB, https://github.com/OpenPCB-app/OpenPCB. Net extraction follows its
union-find approach (T-junctions connect, crossings do not).

## SwiftFlow (MIT)

The canvas interaction model (viewport transform, zoom limits, dot background, Controls, MiniMap,
selection box, theme defaults) is ported from SwiftFlow, https://github.com/aaurelions/SwiftFlow:

```
MIT License

Copyright (c) 2026 A.Aurelions

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## ngspice (BSD-3-Clause, loaded at runtime)

https://ngspice.sourceforge.io — the simulation engine. Windows builds bundle ngspice's official
DLL package (ngspice.dll, XSPICE code models, LLVM OpenMP runtime `libomp140.x86_64.dll`, Apache-2.0
with LLVM exception) plus `sndfile.dll` (libsndfile, LGPL-2.1-or-later) and `samplerate.dll`
(libsamplerate, BSD-2-Clause) from ngspice's console package, all unmodified and loaded at runtime.

## LTspice and Micro-Cap 12 libraries

nomospice does not ship them. It reads an existing local LTspice installation (lib/sym, lib/cmp, lib/sub)
and, on request, downloads the freeware Micro-Cap 12 package (Spectrum Software, 2019) from the
Internet Archive and unpacks its LIBRARY folder locally. The built-in symbols are original artwork with
LTspice-compatible pin positions for .asc interoperability.
