param(
  [string]$MasterExe = 'target/release/void.exe',
  [string]$OutputMsi = 'output/void-template.msi'
)
$ErrorActionPreference = 'Stop'

$wixBin = if (Get-Command candle.exe -ErrorAction SilentlyContinue) {
  Split-Path (Get-Command candle.exe).Source
} else {
  Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'WiX Toolset v3.14/bin'
}
$candle = Join-Path $wixBin 'candle.exe'
$light = Join-Path $wixBin 'light.exe'
if (-not (Test-Path $candle) -or -not (Test-Path $light)) {
  throw 'WiX Toolset 3.14 candle.exe/light.exe are required to build the master MSI template'
}

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "void-msi-tpl-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force $tempDir, (Split-Path -Parent (Join-Path $PWD $OutputMsi)) | Out-Null
try {
  $binaryPath = Join-Path $tempDir 'void.exe'
  $configPath = Join-Path $tempDir 'void.json'
  $iconPath = Join-Path $tempDir 'app.ico'
  Copy-Item -Force $MasterExe $binaryPath
  Copy-Item -Force 'void.json' $configPath
  if (Test-Path 'branding/app.ico') {
    Copy-Item -Force 'branding/app.ico' $iconPath
  } else {
    node scripts/prepare-branding.mjs
    Copy-Item -Force 'branding/app.ico' $iconPath
  }

  $dialogBmp = Join-Path $PWD 'branding/wix-dialog.bmp'
  $bannerBmp = Join-Path $PWD 'branding/wix-banner.bmp'
  $wixUiVars = ''
  if ((Test-Path $dialogBmp) -and (Test-Path $bannerBmp)) {
    $wixUiVars = @'
    <WixVariable Id="WixUIDialogBmp" Value="$(var.DialogBmp)" />
    <WixVariable Id="WixUIBannerBmp" Value="$(var.BannerBmp)" />
'@
  }

  $wxs = @'
<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="*" Name="VOID Engine" Language="1033" Version="1.0.0" Manufacturer="VOID Engine" UpgradeCode="5B73C6A4-83E9-4B43-9D6E-0A9A32A41E01">
    <Package InstallerVersion="500" Compressed="yes" InstallScope="perUser" Platform="x64" />
    <MajorUpgrade DowngradeErrorMessage="A newer version is already installed." />
    <Media Id="1" Cabinet="cab1.cab" EmbedCab="yes" CompressionLevel="none" />
    <Icon Id="ProductIcon" SourceFile="$(var.IconPath)" />
    <Property Id="ARPPRODUCTICON" Value="ProductIcon" />
    <Property Id="WIXUI_INSTALLDIR" Value="INSTALLFOLDER" />
@@WIX_UI_VARS@@
    <UIRef Id="WixUI_InstallDir" />
    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="LocalAppDataFolder">
        <Directory Id="INSTALLFOLDER" Name="void" />
      </Directory>
      <Directory Id="ProgramMenuFolder">
        <Directory Id="StartMenuDir" Name="VOID Engine" />
      </Directory>
      <Directory Id="DesktopFolder" Name="Desktop" />
    </Directory>
    <Feature Id="MainFeature" Title="VOID Engine" Level="1">
      <ComponentRef Id="ApplicationExeComponent" />
      <ComponentRef Id="ApplicationConfigComponent" />
      <ComponentRef Id="StartMenuCleanup" />
      <ComponentRef Id="DesktopShortcutComponent" />
    </Feature>
    <DirectoryRef Id="INSTALLFOLDER">
      <Component Id="ApplicationExeComponent" Guid="8E41A6D2-4F55-4B29-9A63-1D2E3F4A5B6C" Win64="yes">
        <File Id="ApplicationExeFile" Source="$(var.BinaryPath)" Name="void.exe" />
        <RegistryValue Id="RegExeVal" Root="HKCU" Key="Software\VOID Engine\void" Name="Executable" Type="integer" Value="1" KeyPath="yes" />
      </Component>
      <Component Id="ApplicationConfigComponent" Guid="9F52B7E3-5A66-4C3A-8B74-2E3F4A5B6C7D" Win64="yes">
        <File Id="ApplicationConfigFile" Source="$(var.ConfigPath)" Name="void.json" />
        <RegistryValue Id="RegCfgVal" Root="HKCU" Key="Software\VOID Engine\void" Name="Config" Type="integer" Value="1" KeyPath="yes" />
        <RemoveFolder Id="RemoveInstallFolder" Directory="INSTALLFOLDER" On="uninstall" />
      </Component>
    </DirectoryRef>
    <DirectoryRef Id="StartMenuDir">
      <Component Id="StartMenuCleanup" Guid="*" Win64="yes">
        <Shortcut Id="ApplicationShortcut" Name="VOID Engine" Target="[#ApplicationExeFile]" WorkingDirectory="INSTALLFOLDER" Icon="ProductIcon" IconIndex="0" />
        <RemoveFolder Id="RemoveStartMenuFolder" On="uninstall" />
        <RegistryValue Id="RegMenuVal" Root="HKCU" Key="Software\VOID Engine\void" Name="Installed" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>
    <DirectoryRef Id="DesktopFolder">
      <Component Id="DesktopShortcutComponent" Guid="*" Win64="yes">
        <Shortcut Id="DesktopShortcut" Name="VOID Engine" Target="[#ApplicationExeFile]" WorkingDirectory="INSTALLFOLDER" Icon="ProductIcon" IconIndex="0" />
        <RegistryValue Id="RegDeskVal" Root="HKCU" Key="Software\VOID Engine\void" Name="DesktopShortcut" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>
  </Product>
</Wix>
'@
  $wxs = $wxs.Replace('@@WIX_UI_VARS@@', $wixUiVars)
  $wxsPath = Join-Path $tempDir 'template.wxs'
  $wixobjPath = Join-Path $tempDir 'template.wixobj'
  Set-Content -LiteralPath $wxsPath -Value $wxs -Encoding utf8

  $candleArgs = @('-nologo', '-ext', 'WixUIExtension', "-dBinaryPath=$binaryPath", "-dConfigPath=$configPath", "-dIconPath=$iconPath", "-dDialogBmp=$dialogBmp", "-dBannerBmp=$bannerBmp", '-out', $wixobjPath, $wxsPath)
  & $candle @candleArgs
  if ($LASTEXITCODE -ne 0) { throw "WiX candle failed with exit code $LASTEXITCODE" }
  & $light -nologo -sval -ext WixUIExtension -out $OutputMsi $wixobjPath
  if ($LASTEXITCODE -ne 0) { throw "WiX light failed with exit code $LASTEXITCODE" }
  Write-Host "Built master template MSI at $OutputMsi"
} finally {
  Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
}
