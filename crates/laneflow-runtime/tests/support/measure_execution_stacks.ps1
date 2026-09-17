param([Parameter(Mandatory)][string]$ProbeDirectory, [Parameter(Mandatory)][string]$ExpectedExecutable)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or [IntPtr]::Size -ne 8) { throw 'This evidence probe requires 64-bit Windows' }
$probeRoot = [IO.Path]::GetFullPath($ProbeDirectory)
$expectedPath = [IO.Path]::GetFullPath($ExpectedExecutable)
if (-not (Test-Path -LiteralPath $expectedPath -PathType Leaf)) { throw 'Expected test executable is missing' }

# 仅查询被指定测试进程的 TEB 与虚拟页。拒绝布局/身份不符，不把失败记作零字节。
# VirtualQueryEx / MEMORY_BASIC_INFORMATION:
# https://learn.microsoft.com/windows/win32/api/memoryapi/nf-memoryapi-virtualqueryex
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class ExecutionStackProbe {
    [StructLayout(LayoutKind.Sequential)] struct ThreadInfo {
        public int ExitStatus; public IntPtr Teb; public IntPtr Process; public IntPtr Thread;
        public UIntPtr Affinity; public int Priority; public int BasePriority;
    }
    [StructLayout(LayoutKind.Sequential)] struct MemoryInfo {
        public IntPtr BaseAddress; public IntPtr AllocationBase; public uint AllocationProtect;
        public ushort PartitionId; public UIntPtr RegionSize; public uint State; public uint Protect; public uint Type;
    }
    public sealed class StackBytes {
        public int ThreadId; public long ReservedAddressBytes; public long CommittedBytes; public long GuardBytes;
    }
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenThread(uint access, bool inherit, int tid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
    [DllImport("kernel32.dll", SetLastError=true)] static extern UIntPtr VirtualQueryEx(IntPtr process, IntPtr address, out MemoryInfo info, UIntPtr size);
    [DllImport("ntdll.dll")] static extern int NtQueryInformationThread(IntPtr thread, int kind, out ThreadInfo info, int size, out int returned);
    public static StackBytes Read(int pid, int tid) {
        IntPtr process = OpenProcess(0x410, false, pid);
        if (process == IntPtr.Zero) throw new Win32Exception();
        try {
            IntPtr thread = OpenThread(0x40, false, tid);
            if (thread == IntPtr.Zero) throw new Win32Exception();
            try {
                ThreadInfo ti; int returned;
                int status = NtQueryInformationThread(thread, 0, out ti, Marshal.SizeOf<ThreadInfo>(), out returned);
                if (status != 0 || ti.Process.ToInt64() != pid || ti.Thread.ToInt64() != tid) throw new InvalidOperationException("Thread identity/query mismatch");
                byte[] tib = new byte[24]; UIntPtr read;
                if (!ReadProcessMemory(process, ti.Teb, tib, (UIntPtr)tib.Length, out read) || read.ToUInt64() != 24) throw new Win32Exception();
                long stackBase = BitConverter.ToInt64(tib, 8), stackLimit = BitConverter.ToInt64(tib, 16);
                MemoryInfo info;
                if (VirtualQueryEx(process, (IntPtr)stackLimit, out info, (UIntPtr)Marshal.SizeOf<MemoryInfo>()) == UIntPtr.Zero) throw new Win32Exception();
                long allocationBase = info.AllocationBase.ToInt64();
                if (!(allocationBase > 0 && allocationBase <= stackLimit && stackLimit < stackBase && stackBase - allocationBase <= 67108864)) throw new InvalidOperationException("Unexpected native stack layout");
                var result = new StackBytes { ThreadId=tid, ReservedAddressBytes=stackBase-allocationBase };
                for (long address=allocationBase; address<stackBase;) {
                    if (VirtualQueryEx(process, (IntPtr)address, out info, (UIntPtr)Marshal.SizeOf<MemoryInfo>()) == UIntPtr.Zero) throw new Win32Exception();
                    long end = Math.Min(stackBase, checked(info.BaseAddress.ToInt64() + (long)info.RegionSize.ToUInt64()));
                    if (info.AllocationBase.ToInt64()!=allocationBase || end<=address || (info.State!=0x1000 && info.State!=0x2000)) throw new InvalidOperationException("Unexpected stack memory region");
                    if (info.State==0x1000) result.CommittedBytes += end-address;
                    if (info.State==0x1000 && (info.Protect & 0x100)!=0) result.GuardBytes += end-address;
                    address=end;
                }
                return result;
            } finally { CloseHandle(thread); }
        } finally { CloseHandle(process); }
    }
}
'@

$baselineThreads = @()
$workerThreads = @()
$report = @()
foreach ($phase in @('baseline','idle','busy','dropped')) {
    $markerPath = Join-Path $probeRoot "$phase.json"
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    while (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw "Probe phase timeout: $phase" }
        Start-Sleep -Milliseconds 50
    }
    $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
    $process = Get-Process -Id $marker.pid
    if ([IO.Path]::GetFullPath($process.MainModule.FileName) -ne $expectedPath) { throw 'Test process executable mismatch' }
    $threadIds = @($process.Threads | ForEach-Object Id)
    if ($phase -eq 'baseline') { $baselineThreads = $threadIds }
    if ($phase -eq 'idle') {
        $workerThreads = @($threadIds | Where-Object { $_ -notin $baselineThreads })
        if ($workerThreads.Count -ne 3) { throw 'Expected exactly three auxiliary threads' }
    }
    $stacks = @()
    if ($phase -in @('idle','busy')) {
        foreach ($threadId in $workerThreads) { $stacks += [ExecutionStackProbe]::Read($marker.pid, $threadId) }
    }
    if ($phase -eq 'dropped' -and @($workerThreads | Where-Object { $_ -in $threadIds }).Count -ne 0) { throw 'Auxiliary threads survived world drop' }
    $report += [ordered]@{ phase=$phase; pid=$marker.pid; marker=$marker; auxiliaryStacks=$stacks; nativeThreadCount=$threadIds.Count }
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $probeRoot 'native-stack-report.json') -Encoding utf8NoBOM
    [IO.File]::WriteAllText((Join-Path $probeRoot "$phase.ack"), 'observed')
}
$report | ConvertTo-Json -Depth 8
