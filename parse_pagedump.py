"""Parse a Windows PAGEDU64 (paging-file / kernel) dump header.

The PAGEDU64 header is the DUMP_HEADER64 structure. When a kernel dump is
written directly to the pagefile (or a small kernel dump written as a
'PAGEDU64'), it carries the bugcheck code and parameters near the top of the
header.

DUMP_HEADER64 (partial, at 0x0):
  offset 0x00  Signature[4]        "PAGEDU64"
  offset 0x04  ValidDump[4]
  offset 0x08  MajorVersion, MinorVersion (2x ULONG)
  offset 0x10  DirectoryTableBase  ULONG64
  offset 0x18  PfnDataBase         ULONG64
  offset 0x20  PsLoadedModuleList  ULONG64
  offset 0x28  PsActiveProcessHead ULONG64
  offset 0x30  MachineImageType    ULONG
  offset 0x34  NumberProcessors    ULONG
  offset 0x38  BugCheckCode        ULONG
  offset 0x3C  (padding)
  offset 0x40  BugCheckParameter1  ULONG64
  offset 0x48  BugCheckParameter2  ULONG64
  offset 0x50  BugCheckParameter3  ULONG64
  offset 0x58  BugCheckParameter4  ULONG64
"""

import struct
import os

out_path = r"C:\projects\riff\pagedump_analysis.txt"
dump_path = r"C:\Windows\Minidump\090426-18078-01.dmp"

lines = []

def p(s):
    print(s, flush=True)
    lines.append(s)

if not os.path.exists(dump_path):
    p(f"File not found: {dump_path}")
else:
    sz = os.path.getsize(dump_path)
    p(f"Analyzing {dump_path} ({sz} bytes)")
    with open(dump_path, "rb") as f:
        head = f.read(0x68)

        sig = head[0:4]
        if sig not in (b'PAGE', b'PAGEDU64', b'DU64'):
            p(f"Unknown signature: {sig!r}")

        valid_dump  = struct.unpack('<I', head[4:8])[0]
        major_ver   = struct.unpack('<I', head[8:12])[0]
        minor_ver   = struct.unpack('<I', head[12:16])[0]
        # 0x20 PsLoadedModuleList
        ps_loaded_mod = struct.unpack('<Q', head[0x20:0x28])[0]
        # 0x30 MachineImageType
        machine_type = struct.unpack('<I', head[0x30:0x34])[0]
        num_procs    = struct.unpack('<I', head[0x34:0x38])[0]
        bugcheck     = struct.unpack('<I', head[0x3C:0x40])[0] if False else None
        p1 = struct.unpack('<Q', head[0x40:0x48])[0]
        p2 = struct.unpack('<Q', head[0x48:0x50])[0]
        p3 = struct.unpack('<Q', head[0x50:0x58])[0]
        p4 = struct.unpack('<Q', head[0x58:0x60])[0]

        # BugCheckCode location: in DUMP_HEADER64 it's at 0x38 per docs, but
        # MSDN shows BugCheckCode at 0x38 - let's read 0x38 too.
        bugcheck_at_38 = struct.unpack('<I', head[0x38:0x3C])[0]

        p("=== PAGEDU64 Kernel Dump Header ===")
        p(f"  Signature:        {sig!r}")
        p(f"  ValidDump:        0x{valid_dump:08X}  (0x1 = valid crash dump, 0x3 = valid hibernate)")
        p(f"  OS Version:       {major_ver}.{minor_ver}")
        p(f"  PsLoadedModuleList: 0x{ps_loaded_mod:016X}")
        p(f"  MachineImageType: 0x{machine_type:04X}")
        p(f"  NumberProcessors: {num_procs}")
        p(f"  BugCheckCode@38:  0x{bugcheck_at_38:08X}")
        p(f"  Param1:           0x{p1:016X}")
        p(f"  Param2:           0x{p2:016X}")
        p(f"  Param3:           0x{p3:016X}")
        p(f"  Param4:           0x{p4:016X}")

        bugcheck_names = {
            0x00000113: "VIDEO_TDR_FAILURE / VIDEO_DXGKRNL_FATAL_ERROR",
            0x1000007E: "SYSTEM_THREAD_EXCEPTION_NOT_HANDLED",
            0x0000007E: "SYSTEM_THREAD_EXCEPTION_NOT_HANDLED",
            0x00000124: "WHEA_UNCORRECTABLE_ERROR",
            0x00000133: "DPC_WATCHDOG_VIOLATION",
            0x0000009F: "DRIVER_POWER_STATE_FAILURE",
            0x0000000A: "IRQL_NOT_LESS_OR_EQUAL",
            0x000000D1: "DRIVER_IRQL_NOT_LESS_OR_EQUAL",
            0x00000050: "PAGE_FAULT_IN_NONPAGED_AREA",
            0x000000BE: "ATTEMPTED_WRITE_TO_READONLY_MEMORY",
            0x000000F4: "CRITICAL_OBJECT_TERMINATION",
        }

        for bc_name, code in [("code@38", bugcheck_at_38)]:
            if code in bugcheck_names:
                p(f"\n  >>> {bugcheck_names[code]} (bitfield may combine) <<<")

        # The real bugcheck code is parameterized; kernel dumps store the
        # code in BugCheckCode and the 4 params separately. Also try to
        # detect the value even if the 0x38 read seems off by trying
        # alternative offsets (some 64-bit dumps put code at 0x34..0x38).
        p("\n  Interpreting most likely bugcheck code:")
        combined = (bugcheck_at_38 & 0xFFFF)
        p(f"    Primary candidate: 0x{bugcheck_at_38:08X}")

with open(out_path, "w", encoding="utf-8") as f:
    f.write("\n".join(lines))
p(f"\nWrote {out_path}")
