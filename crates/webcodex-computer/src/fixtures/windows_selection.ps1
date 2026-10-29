$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName PresentationFramework
$window = New-Object Windows.Window
$window.Title = 'WebCodex native selection fixture'
$window.Width = 480
$window.Height = 420
$panel = New-Object Windows.Controls.StackPanel
$window.Content = $panel
$list = New-Object Windows.Controls.ListBox
$list.SelectionMode = 'Multiple'
foreach ($name in @('Selection A', 'Selection B', 'Selection disabled')) {
    $item = New-Object Windows.Controls.ListBoxItem
    $item.Content = $name
    [Windows.Automation.AutomationProperties]::SetAutomationId($item, $name)
    [void]$list.Items.Add($item)
}
$list.Items[0].IsSelected = $true
$list.Items[1].IsSelected = $true
$list.Items[2].IsEnabled = $false
$tabs = New-Object Windows.Controls.TabControl
foreach ($name in @('Selection first tab', 'Selection second tab')) {
    $item = New-Object Windows.Controls.TabItem
    $item.Header = $name
    $item.Content = 'Selection fixture content'
    [void]$tabs.Items.Add($item)
}
$tabs.SelectedIndex = 0
$status = New-Object Windows.Controls.TextBlock
$script:changes = 0
function Update-SelectionStatus {
    $status.Text = "selection-business:count=$($list.SelectedItems.Count);tab=$($tabs.SelectedIndex);changes=$script:changes"
}
$list.Add_SelectionChanged({ $script:changes++; Update-SelectionStatus })
$tabs.Add_SelectionChanged({ $script:changes++; Update-SelectionStatus })
Update-SelectionStatus
[void]$panel.Children.Add($list)
[void]$panel.Children.Add($tabs)
[void]$panel.Children.Add($status)
$timer = New-Object Windows.Threading.DispatcherTimer
$timer.Interval = [TimeSpan]::FromSeconds(60)
$timer.Add_Tick({ $window.Close() })
$timer.Start()
[void]$window.ShowDialog()
