function Get-JunctionPowerState {
    @{
        battery = @(Get-CimInstance -Namespace root/wmi -ClassName BatteryStatus -ErrorAction SilentlyContinue | Select-Object PowerOnline,Charging,Discharging)
        powerPlan = @(& powercfg /getactivescheme)
    }
}

function Get-JunctionPowerKey($State) {
    @($State.powerPlan) -join "`n"
    @($State.battery | ForEach-Object { [string]$_.PowerOnline } | Sort-Object) -join ','
}

function Get-JunctionEnvironment {
    # 原始 SMBIOS 值只在内存用于计算，不写入结果或终端。
    $identityParts = @(
        (Get-CimInstance Win32_ComputerSystemProduct).UUID,
        (Get-CimInstance Win32_BIOS).SerialNumber,
        (Get-CimInstance Win32_BaseBoard).SerialNumber
    ) | ForEach-Object { ($_ -replace '\s', '').ToUpperInvariant() }
    $identityBytes = [Text.Encoding]::UTF8.GetBytes($identityParts -join "`n")
    $hardwareDigest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($identityBytes)).ToLowerInvariant()
    $power = Get-JunctionPowerState
    $environment = @{
        hardwareIdentityScheme = 'laneflow-p100-hardware-identity-v2'; hardwareIdentitySha256 = $hardwareDigest
        expectedReferenceMachine = 'LF-P100-REF-01'; hardwareIdentityMatches = $hardwareDigest -eq 'be3637be955f6c2c9e9e55b80419794adfac64b709d573602a37da9a8672fd20'
        os = Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber,TotalVisibleMemorySize
        cpu = Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors
        memory = @(Get-CimInstance Win32_PhysicalMemory | Select-Object Capacity,Speed,ConfiguredClockSpeed)
        bios = Get-CimInstance Win32_BIOS | Select-Object Manufacturer,SMBIOSBIOSVersion,ReleaseDate
        gpu = @(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion)
        battery = $power.battery; powerPlan = $power.powerPlan
        vendorPerformanceMode = 'not programmatically measured'
        rustc = @(& rustc +1.98.0 -Vv); cargo = @(& cargo +1.98.0 -V)
        backgroundProcesses = @(Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First 30 ProcessName,Id,CPU,WorkingSet64)
        certification = 'Uncertified: P10 unspecified; release OS and product memory ceilings not frozen'
    }
    # 固定排序及属性顺序，排除 CPU 使用量、剩余内存和充电状态等动态字段。
    $stable = [ordered]@{
        hardware = $hardwareDigest
        os = @($environment.os.Caption, $environment.os.Version, $environment.os.BuildNumber, $environment.os.TotalVisibleMemorySize)
        cpu = @($environment.cpu | ForEach-Object { "$($_.Name)|$($_.NumberOfCores)|$($_.NumberOfLogicalProcessors)" } | Sort-Object)
        memory = @($environment.memory | ForEach-Object { "$($_.Capacity)|$($_.Speed)|$($_.ConfiguredClockSpeed)" } | Sort-Object)
        bios = @($environment.bios.Manufacturer, $environment.bios.SMBIOSBIOSVersion)
        gpu = @($environment.gpu | ForEach-Object { "$($_.Name)|$($_.DriverVersion)" } | Sort-Object)
    }
    $stableBytes = [Text.Encoding]::UTF8.GetBytes(($stable | ConvertTo-Json -Depth 6 -Compress))
    $environment.stableEnvironmentSha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stableBytes)).ToLowerInvariant()
    $environment
}
