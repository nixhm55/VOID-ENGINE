$ErrorActionPreference = 'Stop'
$appName = if ([string]::IsNullOrWhiteSpace($env:APP_NAME)) { 'VOID Engine' } else { $env:APP_NAME.Trim() }
$appSlug = if ([string]::IsNullOrWhiteSpace($env:APP_SLUG)) { 'void' } else { $env:APP_SLUG.Trim() }
$version = if ([string]::IsNullOrWhiteSpace($env:APP_VERSION)) { '1.0.0' } else { $env:APP_VERSION.Trim() }
if ($appSlug -notmatch '^[a-z0-9][a-z0-9_+-]*$') { throw "Invalid package name: $appSlug" }
$wixVersion = ($version -split '[-+]', 2)[0]
if ($wixVersion -notmatch '^\d{1,5}\.\d{1,5}\.\d{1,5}$') { throw "Invalid MSI version: $version" }

$md5 = [System.Security.Cryptography.MD5]::Create()
try {
  $upgradeCode = '{' + [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes($appSlug))).ToString().ToUpperInvariant() + '}'
  $productCode = '{' + [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("${appSlug}:${wixVersion}:product"))).ToString().ToUpperInvariant() + '}'
  $packageCode = '{' + [Guid]::NewGuid().ToString().ToUpperInvariant() + '}'
  $exeCompGuid = '{' + [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("${appSlug}:exe"))).ToString().ToUpperInvariant() + '}'
  $cfgCompGuid = '{' + [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("${appSlug}:config"))).ToString().ToUpperInvariant() + '}'
} finally {
  $md5.Dispose()
}

$inputDir = Join-Path $PWD.Path 'output\msi-input'
$binaryPath = Join-Path $inputDir "$appSlug.exe"
$configPath = Join-Path $inputDir "$appSlug.json"
$iconPath = Join-Path $inputDir 'app.ico'
$msiPath = Join-Path $PWD.Path "output\$appSlug.msi"
$templateMsi = if ($env:MASTER_TEMPLATE_MSI) { $env:MASTER_TEMPLATE_MSI } else { 'target/master/void-template.msi' }

New-Item -ItemType Directory -Force $inputDir, (Join-Path $PWD.Path 'output') | Out-Null
Copy-Item -Force $env:MASTER_BINARY_PATH $binaryPath
Copy-Item -Force 'void.json' $configPath
Copy-Item -Force 'branding/app.ico' $iconPath
& (Join-Path $PSScriptRoot 'set-windows-icon.ps1') -Executable $binaryPath -IconFile $iconPath

# If the pre-compiled master template MSI was not downloaded from the release yet, build it once
if (-not (Test-Path -LiteralPath $templateMsi)) {
  New-Item -ItemType Directory -Force (Split-Path -Parent $templateMsi) | Out-Null
  & (Join-Path $PSScriptRoot 'build-windows-template-msi.ps1') -MasterExe $binaryPath -OutputMsi $templateMsi
}

Copy-Item -Force $templateMsi $msiPath
Set-ItemProperty -LiteralPath $msiPath -Name IsReadOnly -Value $false -ErrorAction SilentlyContinue

# Pack the icon-stamped executable and void.json into cab1.cab using native makecab.exe (Compress=OFF, MaxDiskSize=0)
$cabDir = Join-Path $inputDir 'cab'
New-Item -ItemType Directory -Force $cabDir | Out-Null
$ddfPath = Join-Path $cabDir 'payload.ddf'
$cabPath = Join-Path $cabDir 'cab1.cab'
$infPath = Join-Path $cabDir 'setup.inf'
$rptPath = Join-Path $cabDir 'setup.rpt'
$ddfContent = @"
.OPTION EXPLICIT
.Set CabinetNameTemplate=cab1.cab
.Set DiskDirectoryTemplate="$cabDir"
.Set DiskDirectory1="$cabDir"
.Set MaxDiskSize=0
.Set Cabinet=ON
.Set Compress=OFF
.Set InfFileName="$infPath"
.Set RptFileName="$rptPath"
"$binaryPath" ApplicationExeFile
"$configPath" ApplicationConfigFile
"@
Set-Content -LiteralPath $ddfPath -Value $ddfContent -Encoding ASCII
$makecabOut = & makecab.exe /F $ddfPath 2>&1
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $cabPath)) {
  throw "makecab.exe failed to build payload cabinet: $($makecabOut -join "`n")"
}

$exeSize = [int](Get-Item -LiteralPath $binaryPath).Length
$cfgSize = [int](Get-Item -LiteralPath $configPath).Length
$shortBase = ($appSlug -replace '[^a-zA-Z0-9]', '').ToUpperInvariant()
if ($shortBase.Length -gt 5) { $shortBase = $shortBase.Substring(0, 5) }
if ([string]::IsNullOrWhiteSpace($shortBase)) { $shortBase = 'VOID' }

# Directly patch the pre-compiled Master MSI database via native Win32 msi.dll API (zero COM/IDispatch overhead)
$msiPatcherSource = @'
using System;
using System.IO;
using System.Runtime.InteropServices;

public static class VoidMsiPatcher {
  private static readonly IntPtr MSIDBOPEN_TRANSACT = new IntPtr(1);
  private const uint VT_LPSTR = 30;

  [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
  private static extern uint MsiOpenDatabaseW(string szDatabasePath, IntPtr szPersist, out IntPtr phDatabase);

  [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
  private static extern uint MsiDatabaseOpenViewW(IntPtr hDatabase, string szQuery, out IntPtr phView);

  [DllImport("msi.dll", ExactSpelling = true)]
  private static extern uint MsiViewExecute(IntPtr hView, IntPtr hRecord);

  [DllImport("msi.dll", ExactSpelling = true)]
  private static extern uint MsiViewClose(IntPtr hView);

  [DllImport("msi.dll", ExactSpelling = true)]
  private static extern IntPtr MsiCreateRecord(uint cParams);

  [DllImport("msi.dll", ExactSpelling = true)]
  private static extern uint MsiRecordSetInteger(IntPtr hRecord, uint iField, int iValue);

  [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
  private static extern uint MsiRecordSetStringW(IntPtr hRecord, uint iField, string szValue);

  [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
  private static extern uint MsiRecordSetStreamW(IntPtr hRecord, uint iField, string szFilePath);

  [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
  private static extern uint MsiGetSummaryInformationW(IntPtr hDatabase, string szDatabasePath, uint uiUpdateCount, out IntPtr phSummaryInfo);

  [DllImport("msi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
  private static extern uint MsiSummaryInfoSetPropertyW(IntPtr hSummaryInfo, uint uiProperty, uint uiDataType, int iValue, IntPtr pftValue, string szValue);

  [DllImport("msi.dll", ExactSpelling = true)]
  private static extern uint MsiSummaryInfoPersist(IntPtr hSummaryInfo);

  [DllImport("msi.dll", ExactSpelling = true)]
  private static extern uint MsiDatabaseCommit(IntPtr hDatabase);

  [DllImport("msi.dll", ExactSpelling = true)]
  private static extern uint MsiCloseHandle(IntPtr hAny);

  private static void Check(uint code, string op) {
    if (code != 0) throw new InvalidOperationException(op + " failed with MSI error code " + code);
  }

  public static IntPtr OpenTransact(string msiPath) {
    string fullPath = Path.GetFullPath(msiPath);
    File.SetAttributes(fullPath, FileAttributes.Normal);
    IntPtr hDb;
    Check(MsiOpenDatabaseW(fullPath, MSIDBOPEN_TRANSACT, out hDb), "MsiOpenDatabaseW");
    return hDb;
  }

  public static void ExecSql(IntPtr hDb, string sql, object[] args) {
    IntPtr hView = IntPtr.Zero;
    IntPtr hRec = IntPtr.Zero;
    try {
      Check(MsiDatabaseOpenViewW(hDb, sql, out hView), "MsiDatabaseOpenViewW: " + sql);
      if (args != null && args.Length > 0) {
        hRec = MsiCreateRecord((uint)args.Length);
        if (hRec == IntPtr.Zero) throw new InvalidOperationException("MsiCreateRecord failed");
        for (int i = 0; i < args.Length; i++) {
          object val = args[i];
          if (val is System.Management.Automation.PSObject pso) val = pso.BaseObject;
          uint field = (uint)(i + 1);
          if (val is int intVal) {
            Check(MsiRecordSetInteger(hRec, field, intVal), "MsiRecordSetInteger");
          } else if (val is long longVal) {
            Check(MsiRecordSetInteger(hRec, field, checked((int)longVal)), "MsiRecordSetInteger");
          } else {
            Check(MsiRecordSetStringW(hRec, field, Convert.ToString(val) ?? string.Empty), "MsiRecordSetStringW");
          }
        }
      }
      Check(MsiViewExecute(hView, hRec), "MsiViewExecute: " + sql);
      Check(MsiViewClose(hView), "MsiViewClose");
    } finally {
      if (hRec != IntPtr.Zero) MsiCloseHandle(hRec);
      if (hView != IntPtr.Zero) MsiCloseHandle(hView);
    }
  }

  public static void ExecStream(IntPtr hDb, string sql, string filePath) {
    IntPtr hView = IntPtr.Zero;
    IntPtr hRec = IntPtr.Zero;
    try {
      Check(MsiDatabaseOpenViewW(hDb, sql, out hView), "MsiDatabaseOpenViewW stream: " + sql);
      hRec = MsiCreateRecord(1);
      if (hRec == IntPtr.Zero) throw new InvalidOperationException("MsiCreateRecord failed");
      Check(MsiRecordSetStreamW(hRec, 1, Path.GetFullPath(filePath)), "MsiRecordSetStreamW");
      Check(MsiViewExecute(hView, hRec), "MsiViewExecute stream: " + sql);
      Check(MsiViewClose(hView), "MsiViewClose stream");
    } finally {
      if (hRec != IntPtr.Zero) MsiCloseHandle(hRec);
      if (hView != IntPtr.Zero) MsiCloseHandle(hView);
    }
  }

  public static void UpdateSummaryAndCommit(IntPtr hDb, string appName, string packageCode) {
    IntPtr hSum = IntPtr.Zero;
    try {
      Check(MsiGetSummaryInformationW(hDb, null, 10, out hSum), "MsiGetSummaryInformationW");
      Check(MsiSummaryInfoSetPropertyW(hSum, 2, VT_LPSTR, 0, IntPtr.Zero, appName), "MsiSummaryInfoSetPropertyW(Title)");
      Check(MsiSummaryInfoSetPropertyW(hSum, 3, VT_LPSTR, 0, IntPtr.Zero, appName), "MsiSummaryInfoSetPropertyW(Subject)");
      Check(MsiSummaryInfoSetPropertyW(hSum, 9, VT_LPSTR, 0, IntPtr.Zero, packageCode), "MsiSummaryInfoSetPropertyW(PackageCode)");
      Check(MsiSummaryInfoPersist(hSum), "MsiSummaryInfoPersist");
    } finally {
      if (hSum != IntPtr.Zero) MsiCloseHandle(hSum);
    }
    Check(MsiDatabaseCommit(hDb), "MsiDatabaseCommit");
  }

  public static void Close(IntPtr hDb) {
    if (hDb != IntPtr.Zero) MsiCloseHandle(hDb);
  }
}
'@

if (-not ('VoidMsiPatcher' -as [type])) {
  Add-Type -TypeDefinition $msiPatcherSource -Language CSharp
}

$hDb = [VoidMsiPatcher]::OpenTransact($msiPath)
try {
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Property` SET `Value` = ? WHERE `Property` = ''ProductName''', [object[]]@([string]$appName))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Property` SET `Value` = ? WHERE `Property` = ''ProductVersion''', [object[]]@([string]$wixVersion))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Property` SET `Value` = ? WHERE `Property` = ''ProductCode''', [object[]]@([string]$productCode))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Property` SET `Value` = ? WHERE `Property` = ''UpgradeCode''', [object[]]@([string]$upgradeCode))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Upgrade` SET `UpgradeCode` = ?', [object[]]@([string]$upgradeCode))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Component` SET `ComponentId` = ? WHERE `Component` = ''ApplicationExeComponent''', [object[]]@([string]$exeCompGuid))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Component` SET `ComponentId` = ? WHERE `Component` = ''ApplicationConfigComponent''', [object[]]@([string]$cfgCompGuid))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Directory` SET `DefaultDir` = ? WHERE `Directory` = ''INSTALLFOLDER''', [object[]]@([string]"${shortBase}DIR|$appSlug"))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Directory` SET `DefaultDir` = ? WHERE `Directory` = ''StartMenuDir''', [object[]]@([string]"${shortBase}MNU|$appName"))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `File` SET `FileName` = ?, `FileSize` = ? WHERE `File` = ''ApplicationExeFile''', [object[]]@([string]"${shortBase}EXE.EXE|$appSlug.exe", [int]$exeSize))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `File` SET `FileName` = ?, `FileSize` = ? WHERE `File` = ''ApplicationConfigFile''', [object[]]@([string]"${shortBase}CFG.JSN|$appSlug.json", [int]$cfgSize))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Shortcut` SET `Name` = ? WHERE `Shortcut` = ''ApplicationShortcut''', [object[]]@([string]"${shortBase}LNK|$appName"))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Shortcut` SET `Name` = ? WHERE `Shortcut` = ''DesktopShortcut''', [object[]]@([string]"${shortBase}DSK|$appName"))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Registry` SET `Key` = ? WHERE `Registry` = ''RegExeVal''', [object[]]@([string]"Software\VOID Engine\$appSlug"))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Registry` SET `Key` = ? WHERE `Registry` = ''RegCfgVal''', [object[]]@([string]"Software\VOID Engine\$appSlug"))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Registry` SET `Key` = ? WHERE `Registry` = ''RegMenuVal''', [object[]]@([string]"Software\VOID Engine\$appSlug"))
  [VoidMsiPatcher]::ExecSql($hDb, 'UPDATE `Registry` SET `Key` = ? WHERE `Registry` = ''RegDeskVal''', [object[]]@([string]"Software\VOID Engine\$appSlug"))

  [VoidMsiPatcher]::ExecStream($hDb, 'UPDATE `Icon` SET `Data` = ? WHERE `Name` = ''ProductIcon''', [string]$iconPath)
  [VoidMsiPatcher]::ExecStream($hDb, 'UPDATE `_Streams` SET `Data` = ? WHERE `Name` = ''cab1.cab''', [string]$cabPath)

  [VoidMsiPatcher]::UpdateSummaryAndCommit($hDb, [string]$appName, [string]$packageCode)
} finally {
  [VoidMsiPatcher]::Close($hDb)
}

Remove-Item -Recurse -Force $inputDir -ErrorAction SilentlyContinue
Write-Host "Packaged $appName as $msiPath"
