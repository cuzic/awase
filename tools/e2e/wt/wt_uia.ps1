# Windows Terminal の TermControl を UI Automation の TextPattern で読み、画面のテキストを標準出力に出す。
# 使い方: pwsh -File wt_uia.ps1 -Hwnd <最上位窓ハンドル(10進)> [-CountTabs]
param([Parameter(Mandatory = $true)][long]$Hwnd, [switch]$CountTabs)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Hwnd)
if ($CountTabs) {
  $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::TabItem)
  $tabs = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
  "TABS=$($tabs.Count)"
  return
}
$all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
$found = 0
foreach ($el in $all) {
  $pat = $null
  if ($el.TryGetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern, [ref]$pat)) {
    $found++
    "TEXTPATTERN class=$($el.Current.ClassName) name=$($el.Current.Name)"
    $pat.DocumentRange.GetText(20000)
    "---END---"
  }
}
"TEXTPATTERN_COUNT=$found"
