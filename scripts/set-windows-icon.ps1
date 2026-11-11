param(
  [Parameter(Mandatory = $true)][string]$Executable,
  [Parameter(Mandatory = $true)][string]$IconFile
)
$ErrorActionPreference = 'Stop'

$source = @'
using System;
using System.IO;
using System.Runtime.InteropServices;
public static class VoidPeIcon {
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
  static extern IntPtr BeginUpdateResource(string file, bool deleteExisting);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool UpdateResource(IntPtr update, IntPtr type, IntPtr name, ushort language, byte[] data, uint size);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool EndUpdateResource(IntPtr update, bool discard);
  static IntPtr Id(int value) { return new IntPtr(value); }
  static void Check(bool ok, string operation) {
    if (!ok) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), operation);
  }
  public static void Apply(string executable, string iconPath) {
    byte[] ico = File.ReadAllBytes(iconPath);
    if (ico.Length < 22 || BitConverter.ToUInt16(ico, 0) != 0 || BitConverter.ToUInt16(ico, 2) != 1)
      throw new InvalidDataException("Invalid ICO file");
    int count = BitConverter.ToUInt16(ico, 4);
    if (count == 0 || ico.Length < 6 + count * 16) throw new InvalidDataException("ICO has no images");
    byte[] group = new byte[6 + count * 14];
    Buffer.BlockCopy(ico, 0, group, 0, 6);
    IntPtr update = BeginUpdateResource(Path.GetFullPath(executable), false);
    if (update == IntPtr.Zero) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), "BeginUpdateResource");
    bool committed = false;
    try {
      for (int i = 0; i < count; i++) {
        int entry = 6 + i * 16;
        int size = checked((int)BitConverter.ToUInt32(ico, entry + 8));
        int offset = checked((int)BitConverter.ToUInt32(ico, entry + 12));
        if (size <= 0 || offset < 0 || offset + size > ico.Length) throw new InvalidDataException("ICO image entry is out of bounds");
        byte[] image = new byte[size];
        Buffer.BlockCopy(ico, offset, image, 0, size);
        ushort imageId = checked((ushort)(i + 1));
        Check(UpdateResource(update, Id(3), Id(imageId), 0, image, (uint)image.Length), "Update RT_ICON resource");
        int groupEntry = 6 + i * 14;
        Buffer.BlockCopy(ico, entry, group, groupEntry, 12);
        Buffer.BlockCopy(BitConverter.GetBytes(imageId), 0, group, groupEntry + 12, 2);
      }
      Check(UpdateResource(update, Id(14), Id(1), 0, group, (uint)group.Length), "Update RT_GROUP_ICON resource");
      Check(EndUpdateResource(update, false), "EndUpdateResource");
      committed = true;
    } finally {
      if (!committed) EndUpdateResource(update, true);
    }
  }
}
'@

if (-not ('VoidPeIcon' -as [type])) { Add-Type -TypeDefinition $source -Language CSharp }
[VoidPeIcon]::Apply((Resolve-Path -LiteralPath $Executable).Path, (Resolve-Path -LiteralPath $IconFile).Path)
