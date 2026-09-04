"""Parse the Windows minidump to extract the bugcheck info."""

import struct
import sys
import os

# Minidump stream types
MiniDumpStreamType = {
    0: "UnusedStream",
    1: "ReservedStream0",
    2: "OptionalStream0",
    3: "OptionalStream1",
    4: "OptionalStream2",
    5: "OptionalStream3",
    6: "OptionalStream4",
    7: "OptionalStream5",
    8: "OptionalStream6",
    9: "OptionalStream7",
    10: "OptionalStream8",
    11: "OptionalStream9",
    12: "OptionalStream10",
    13: "OptionalStream11",
    14: "CommentStreamA",
    15: "CommentStreamW",
    16: "HandleDataStream",
    17: "MemoryInfoListStream",
    18: "ThreadInfoListStream",
    19: "HandleOperationListStream",
    20: "TokenStream",
    100: "LinuxProcInfoStream",
    200: "LinuxAuxvStream",
    201: "LinuxCmdLineInfoStream",
    202: "LinuxCpuInfoStream",
    203: "LinuxProcStatusStream",
    111: "JavaScriptInfoStream",
    4096: "SystemInfoStream",
    4097: "ThreadListStream",
    4098: "ModuleListStream",
    4099: "MemoryListStream",
    4100: "ExceptionStream",
    4101: "SystemExInfoStream",
    4102: "AcpiTableStream",
    4103: "EmergencyCondRecord",
    4104: "BiasInfoStream",
    4105: "PlatformIdStream",
    4106: "ProcessorInfoStream",
    4107: "VSMStateInfoStream",
    4108: "InstrumentationInfoStream",
    4109: "ObjectDirectoryStream",
    4110: "PowerInfoStream",
    4111: "ProcessVmCountersStream",
    4112: "IptTraceStream",
    4113: "ThreadNamesStream",
    4114: "CEUMDBInfoStream",
    4115: "PbiPublicStream",
    4116: "PbiPrivateStream",
    4117: "WinpointInfoStream",
    4118: "KernelMDmpInformationStream",
}

def read_minidump_header(f):
    f.seek(0)
    sig = f.read(4)
    if sig != b'MDMP':
        print(f"Not a minidump! Signature: {sig}")
        return None

    version          = struct.unpack('<I', f.read(4))[0]
    num_streams     = struct.unpack('<I', f.read(4))[0]
    dir_rva         = struct.unpack('<I', f.read(4))[0]
    checksum        = struct.unpack('<I', f.read(4))[0]
    timestamp       = struct.unpack('<I', f.read(4))[0]
    flags           = struct.unpack('<Q', f.read(8))[0]

    print(f"Minidump Version:        {version}")
    print(f"Number of streams:       {num_streams}")
    print(f"Stream directory RVA:    0x{dir_rva:X}")
    print(f"Checksum:                0x{checksum:08X}")
    print(f"Timestamp:               {timestamp}")
    print(f"Flags:                   0x{flags:016X}")
    print()
    return dict(version=version, number_of_streams=num_streams,
                stream_directory_rva=dir_rva, checksum=checksum,
                timestamp=timestamp, flags=flags)


def read_locationDescriptor(f):
    """MINIDUMP_LOCATION_DESCRIPTOR"""
    dataSize = struct.unpack('<I', f.read(4))[0]
    rva      = struct.unpack('<I', f.read(4))[0]
    return dataSize, rva


def parse_exception_stream(f, rva, dataSize):
    f.seek(rva)

    thread_id       = struct.unpack('<I', f.read(4))[0]
    f.read(4)                     # alignment padding
    exception_code  = struct.unpack('<I', f.read(4))[0]
    exception_flags = struct.unpack('<I', f.read(4))[0]
    f.read(8)                     # ExceptionRecord (pointer, unused)
    exception_addr  = struct.unpack('<Q', f.read(8))[0]
    num_params      = struct.unpack('<I', f.read(4))[0]
    f.read(4)                     # __alignment

    params = []
    for _ in range(min(num_params, 15)):
        params.append(struct.unpack('<Q', f.read(8))[0])

    print("=== Exception Stream ===")
    print(f"  Thread ID:         {thread_id}")
    print(f"  Exception Code:    0x{exception_code:08X}")
    print(f"  Exception Flags:   0x{exception_flags:X}")
    print(f"  Exception Address: 0x{exception_addr:016X}")
    print(f"  Number Parameters: {num_params}")

    bugcheck_names = {
        0x00000113: "VIDEO_TDR_FAILURE / VIDEO_DXGKRNL_FATAL_ERROR",
        0x00000124: "WHEA_UNCORRECTABLE_ERROR",
        0x00000133: "DPC_WATCHDOG_VIOLATION",
        0x0000009F: "DRIVER_POWER_STATE_FAILURE",
        0x0000000A: "IRQL_NOT_LESS_OR_EQUAL",
        0x000000D1: "DRIVER_IRQL_NOT_LESS_OR_EQUAL",
        0x0000007E: "SYSTEM_THREAD_EXCEPTION_NOT_HANDLED",
        0x00000050: "PAGE_FAULT_IN_NONPAGED_AREA",
        0x000000BE: "ATTEMPTED_WRITE_TO_READONLY_MEMORY",
        0x000000F4: "CRITICAL_OBJECT_TERMINATION",
    }

    if exception_code in bugcheck_names:
        print(f"\n  >>> {bugcheck_names[exception_code]} <<<")

    if exception_code == 0x113 and len(params) >= 4:
        print(f"\n  TDR Driver Object:  0x{params[0]:016X}")
        print(f"  TDR Subcode:       0x{params[1]:X}  (=2 means TDR detected non-completion)")
        dev = params[3]
        vendor_id   = (dev >> 16) & 0xFFFF
        device_id   = dev & 0xFFFF
        subsystem   = (dev >> 32) & 0xFFFF
        revision    = (dev >> 48) & 0xFFFF
        vendor_names = {0x10DE: "NVIDIA", 0x1002: "AMD/ATI", 0x8086: "Intel"}
        print(f"  Device ID:         0x{dev:016X}")
        print(f"    Vendor ID:       0x{vendor_id:04X}  {vendor_names.get(vendor_id, '?')}")
        print(f"    Device ID:       0x{device_id:04X}")
        print(f"    Subsystem:       0x{subsystem:04X}")
        print(f"    Revision:        0x{revision:04X}")

    for i, p in enumerate(params):
        print(f"  Param[{i}]: 0x{p:016X}")

    return params


def parse_system_info_stream(f, rva, dataSize):
    f.seek(rva)

    arch          = struct.unpack('<H', f.read(2))[0]
    level         = struct.unpack('<H', f.read(2))[0]
    revision      = struct.unpack('<H', f.read(2))[0]
    num_procs     = struct.unpack('B', f.read(1))[0]
    product_type  = struct.unpack('B', f.read(1))[0]
    major_ver     = struct.unpack('<I', f.read(4))[0]
    minor_ver     = struct.unpack('<I', f.read(4))[0]
    build_num     = struct.unpack('<I', f.read(4))[0]
    platform_id   = struct.unpack('<I', f.read(4))[0]
    csd_rva       = struct.unpack('<I', f.read(4))[0]
    suite_mask    = struct.unpack('<H', f.read(2))[0]
    reserved      = struct.unpack('<H', f.read(2))[0]
    f.read(8)                    # processor_features[2]

    arch_map = {0: "x86", 5: "ARM", 6: "IA64", 9: "AMD64", 12: "ARM64", 0xAA64: "ARM64"}
    print("\n=== System Info ===")
    print(f"  Processor Arch:    {arch_map.get(arch, str(arch))}")
    print(f"  Processor Level:   {level}")
    print(f"  Number of CPUs:    {num_procs}")
    print(f"  Product Type:     {product_type}  (1=workstation, 2=domain controller, 3=server)")
    print(f"  OS Version:       {major_ver}.{minor_ver}.{build_num}")
    print(f"  Platform ID:       {platform_id}  (2=NT, 1=WIN95)")
    print(f"  Suite Mask:        0x{suite_mask:04X}")
    print(f"  CSD RVA:           0x{csd_rva:X}")

    if csd_rva:
        f.seek(csd_rva)
        csd_len = struct.unpack('<I', f.read(4))[0]
        csd_str = f.read(csd_len * 2).decode('utf-16-le').rstrip('\x00')
        print(f"  CSD Version:       '{csd_str}'")


def parse_module_list(f, rva, dataSize):
    """List loaded modules (drivers) from the module list stream."""
    f.seek(rva)
    num_modules = struct.unpack('<I', f.read(4))[0]
    print(f"\n=== Module List ({num_modules} modules) ===")
    for i in range(num_modules):
        base_of_image = struct.unpack('<Q', f.read(8))[0]
        size_of_image  = struct.unpack('<I', f.read(4))[0]
        checksum       = struct.unpack('<I', f.read(4))[0]
        timestamp      = struct.unpack('<I', f.read(4))[0]
        module_name_rva= struct.unpack('<I', f.read(4))[0]
        f.read(16)                    # version_info (VS_FIXEDFILEINFO)
        f.read(8)                    # cv_record (CodeView PDB info)
        f.read(8)                    # misc_record
        f.read(8)                    # (2x reserved)

        # Try to read module name
        saved_pos = f.tell()
        f.seek(module_name_rva)
        name_len = struct.unpack('<I', f.read(4))[0]
        if name_len > 0 and name_len < 512:
            name_bytes = f.read(name_len * 2)
            try:
                name = name_bytes.decode('utf-16-le').rstrip('\x00')
            except:
                name = "<unreadable>"
        else:
            name = "<invalid>"
        f.seek(saved_pos)

        if base_of_image != 0:
            print(f"  0x{base_of_image:016X}  0x{size_of_image:08X}  {name}")


def main():
    dump_path = r"C:\Windows\Minidump\090426-18078-01.dmp"
    out_path  = r"C:\projects\riff\dump_analysis.txt"

    import builtins
    _orig_print = builtins.print
    _outf = open(out_path, "w", encoding="utf-8")
    def tee_print(*args, **kwargs):
        sep = kwargs.get("sep", " ")
        line = sep.join(str(a) for a in args) + kwargs.get("end", "\n")
        _orig_print(line, end="", flush=True)
        _outf.write(line)
        _outf.flush()
    builtins.print = tee_print

    try:
        if not os.path.exists(dump_path):
            print(f"Dump file not found: {dump_path}")
            return

        with open(dump_path, "rb") as f:
            header = read_minidump_header(f)
            if not header:
                return

            print("\n--- Stream Directory ---")
            f.seek(header["stream_directory_rva"])

            stream_info = []
            for i in range(header["number_of_streams"]):
                stype = struct.unpack("<I", f.read(4))[0]
                dsz   = struct.unpack("<I", f.read(4))[0]
                rva   = struct.unpack("<I", f.read(4))[0]
                name  = MiniDumpStreamType.get(stype, f"Unknown({stype})")
                print(f"  Stream {i:2d}: {name:40s} type={stype:4d} size=0x{dsz:06X} RVA=0x{rva:08X}")
                stream_info.append((stype, dsz, rva))

            for stype, dsz, rva in stream_info:
                if stype == 4100:
                    print()
                    parse_exception_stream(f, rva, dsz)
                elif stype == 4096:
                    parse_system_info_stream(f, rva, dsz)
                elif stype == 4098:
                    parse_module_list(f, rva, dsz)
    finally:
        builtins.print = _orig_print
        _outf.close()
        _orig_print(f"\nOutput written to {out_path}")

if __name__ == "__main__":
    main()
