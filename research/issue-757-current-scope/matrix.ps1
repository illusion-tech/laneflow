param([string]$OutputRoot='target/review-screen')
$ErrorActionPreference='Stop'
$order=@('all','all','p3','waiting','both','both','waiting','p3','all')
foreach ($scale in @('10k','100k')) {
    for ($i=0; $i -lt $order.Count; $i++) {
        & "$PSScriptRoot/run.ps1" -Binary target/binaries/scope-plain-review.exe -Source target/review-source -OutputRoot $OutputRoot -Label "$scale-$($i+1)-$($order[$i])" -Scale $scale -Mode $order[$i] -Ticks 512
    }
}
