# Keep the Process handle alive through the final read. PID-based .NET properties
# cannot reliably refresh a process after exit.
# https://learn.microsoft.com/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo
if (-not ('JunctionProcessMemory' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class JunctionProcessMemory {
    [StructLayout(LayoutKind.Sequential)]
    public struct Counters {
        public uint cb, PageFaultCount;
        public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage,
            QuotaPagedPoolUsage, QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage,
            PagefileUsage, PeakPagefileUsage, PrivateUsage;
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool K32GetProcessMemoryInfo(SafeProcessHandle process,
        ref Counters counters, uint size);
    public static Counters Read(SafeProcessHandle process) {
        var counters = new Counters { cb = (uint)Marshal.SizeOf<Counters>() };
        if (!K32GetProcessMemoryInfo(process, ref counters, counters.cb))
            throw new Win32Exception(Marshal.GetLastWin32Error());
        return counters;
    }
}
'@
}
