// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text.RegularExpressions;

namespace Microsoft.Mxc.PolicyCatalog;

/// <summary>
/// The default host environment (TypeScript <c>nodeHostEnvironment</c>). The native architecture is detected
/// once per process, on first need, and never taken from the process or compile-time architecture.
/// </summary>
public sealed class SystemHostEnvironment : IHostEnvironment
{
    private static readonly Lazy<string> NativeArchitectureCache = new(DetectNativeArchitecture, LazyThreadSafetyMode.ExecutionAndPublication);

    private SystemHostEnvironment()
    {
    }

    /// <summary>The shared instance.</summary>
    public static SystemHostEnvironment Instance { get; } = new();

    /// <inheritdoc />
    public string Platform()
    {
        if (OperatingSystem.IsWindows())
        {
            return CatalogPlatforms.Windows;
        }

        if (OperatingSystem.IsMacOS())
        {
            return CatalogPlatforms.MacOS;
        }

        if (OperatingSystem.IsLinux())
        {
            return CatalogPlatforms.Linux;
        }

        throw new PolicyCatalogException(PolicyCatalogErrorReason.UnsupportedHost, $"host platform '{NodePlatformName()}' has no catalog selector");
    }

    /// <inheritdoc />
    public string NativeArchitecture()
    {
        // A failed detection is not cached as a value; Lazy caches the exception, matching a
        // deterministic host fact for the process lifetime.
        return NativeArchitectureCache.Value;
    }

    /// <inheritdoc />
    public string? Symbol(string name)
    {
        switch (name)
        {
            case "user_home":
                return NonEmpty(HomeDirectory());
            case "temp_dir":
                return NonEmpty(TempDirectory());
            default:
                return null;
        }
    }

    private static string? NonEmpty(string? value) => string.IsNullOrEmpty(value) ? null : value;

    // Node os.homedir(): USERPROFILE on Windows, HOME on POSIX, then the account database.
    private static string? HomeDirectory()
    {
        var fromEnvironment = Environment.GetEnvironmentVariable(OperatingSystem.IsWindows() ? "USERPROFILE" : "HOME");
        return string.IsNullOrEmpty(fromEnvironment) ? Environment.GetFolderPath(Environment.SpecialFolder.UserProfile) : fromEnvironment;
    }

    // Node os.tmpdir(): Windows TEMP/TMP/%SystemRoot%\temp with a trailing '\' removed unless after ':';
    // POSIX TMPDIR/TMP/TEMP or /tmp with a trailing '/' removed.
    private static string TempDirectory()
    {
        if (OperatingSystem.IsWindows())
        {
            var path = FirstEnv("TEMP", "TMP") ?? ((Environment.GetEnvironmentVariable("SystemRoot") ?? Environment.GetEnvironmentVariable("windir")) + "\\temp");
            if (path.Length > 1 && path[path.Length - 1] == '\\' && path[path.Length - 2] != ':')
            {
                path = path.Substring(0, path.Length - 1);
            }

            return path;
        }

        var posix = FirstEnv("TMPDIR", "TMP", "TEMP") ?? "/tmp";
        if (posix.Length > 1 && posix[posix.Length - 1] == '/')
        {
            posix = posix.Substring(0, posix.Length - 1);
        }

        return posix;
    }

    private static string? FirstEnv(params string[] names)
    {
        foreach (var name in names)
        {
            var value = Environment.GetEnvironmentVariable(name);
            if (!string.IsNullOrEmpty(value))
            {
                return value;
            }
        }

        return null;
    }

    private static string NodePlatformName()
    {
        if (OperatingSystem.IsFreeBSD())
        {
            return "freebsd";
        }

        return RuntimeInformation.OSDescription;
    }

    /// <summary>Maps an OS-reported machine/architecture string to a catalog selector (TypeScript <c>architectureFromMachine</c>).</summary>
    /// <param name="machine">For example <c>AMD64</c>, <c>x86_64</c>, <c>ARM64</c>, <c>aarch64</c>.</param>
    /// <returns><c>x64</c>, <c>arm64</c>, or <c>null</c> when unknown.</returns>
    public static string? ArchitectureFromMachine(string machine)
    {
        switch (Internal.JsString.ToLower(Internal.JsString.Trim(machine)))
        {
            case "x86_64":
            case "amd64":
            case "x64":
                return CatalogArchitectures.X64;
            case "arm64":
            case "aarch64":
                return CatalogArchitectures.Arm64;
            default:
                return null;
        }
    }

    /// <summary>
    /// Windows: the machine-wide <c>PROCESSOR_ARCHITECTURE</c> under
    /// <c>HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment</c> (registry API; an emulated
    /// process still reads the native value). macOS: <c>/usr/sbin/sysctl -n hw.optional.arm64</c> (absent key
    /// = Intel), then <c>/usr/bin/uname -m</c>. Linux: <c>/usr/bin/uname -m</c> (or <c>/bin/uname</c>), the
    /// kernel machine type. Executables are invoked by absolute path.
    /// </summary>
    private static string DetectNativeArchitecture()
    {
        string? reported;
        try
        {
            if (OperatingSystem.IsWindows())
            {
                reported = ReadWindowsRegistryArchitecture();
            }
            else if (OperatingSystem.IsMacOS())
            {
                var appleSilicon = false;
                try
                {
                    appleSilicon = Run("/usr/sbin/sysctl", "-n", "hw.optional.arm64").Trim() == "1";
                }
                catch (Exception)
                {
                    // The key is absent on Intel Macs.
                }

                reported = appleSilicon ? "arm64" : Run("/usr/bin/uname", "-m").Trim();
            }
            else
            {
                var uname = File.Exists("/usr/bin/uname") ? "/usr/bin/uname" : "/bin/uname";
                reported = Run(uname, "-m").Trim();
            }
        }
        catch (Exception error)
        {
            throw new PolicyCatalogException(PolicyCatalogErrorReason.UnsupportedHost, $"native system architecture could not be determined: {error.Message}");
        }

        var architecture = reported is null ? null : ArchitectureFromMachine(reported);
        return architecture
            ?? throw new PolicyCatalogException(PolicyCatalogErrorReason.UnsupportedHost, $"native system architecture '{reported ?? "unknown"}' has no catalog selector");
    }

    [System.Runtime.Versioning.SupportedOSPlatform("windows")]
    private static string? ReadWindowsRegistryArchitecture()
    {
        // RegistryView.Default from a 64-bit process, and Registry64 from a 32-bit one, both see the
        // machine-wide value; this key is not redirected by WOW64 anyway.
        using var hive = Microsoft.Win32.RegistryKey.OpenBaseKey(Microsoft.Win32.RegistryHive.LocalMachine, Microsoft.Win32.RegistryView.Registry64);
        using var key = hive.OpenSubKey(@"SYSTEM\CurrentControlSet\Control\Session Manager\Environment")
            ?? throw new InvalidOperationException(@"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment could not be opened");
        var value = key.GetValue("PROCESSOR_ARCHITECTURE", null, Microsoft.Win32.RegistryValueOptions.DoNotExpandEnvironmentNames) as string;
        // reg.exe output is matched with \S+ in the TypeScript reference; keep the first token.
        return value is null ? null : Regex.Match(value, "^\\S*").Value;
    }

    private static string Run(string file, params string[] args)
    {
        var start = new ProcessStartInfo(file)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            RedirectStandardInput = true,
            UseShellExecute = false,
            CreateNoWindow = true,
        };
        foreach (var arg in args)
        {
            start.ArgumentList.Add(arg);
        }

        using var process = Process.Start(start) ?? throw new InvalidOperationException($"{file} could not be started");
        process.StandardInput.Close();
        var stderr = process.StandardError.ReadToEndAsync();
        var output = process.StandardOutput.ReadToEnd();
        process.WaitForExit();
        _ = stderr.Result;
        if (process.ExitCode != 0)
        {
            throw new InvalidOperationException($"Command failed: {file} {string.Join(" ", args)}");
        }

        return output;
    }
}
