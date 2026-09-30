// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text;
using System.Text.RegularExpressions;

namespace Microsoft.Mxc.PolicyCatalog.Internal;

/// <summary>
/// Ports of Node's <c>path.win32</c> / <c>path.posix</c> <c>normalize</c>, <c>isAbsolute</c>, and
/// <c>parse().root</c> (node v24.20.0 <c>lib/path.js</c>), plus the catalog's path rules.
/// </summary>
internal static class Paths
{
    private static readonly string[] WindowsReservedNames =
    {
        "CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        "COM\u00b9", "COM\u00b2", "COM\u00b3",
        "LPT\u00b9", "LPT\u00b2", "LPT\u00b3",
    };

    private static readonly Regex TrailingSeparators = new("[\\\\/]+\\z", RegexOptions.CultureInvariant);
    private static readonly Regex WindowsSeparators = new("[\\\\/]+", RegexOptions.CultureInvariant);
    private static readonly Regex PosixSeparators = new("/+", RegexOptions.CultureInvariant);

    /// <summary>JavaScript <c>String.prototype.slice</c>.</summary>
    internal static string Slice(string value, int start, int? end = null)
    {
        var length = value.Length;
        var from = start < 0 ? Math.Max(length + start, 0) : Math.Min(start, length);
        var e = end ?? length;
        var to = e < 0 ? Math.Max(length + e, 0) : Math.Min(e, length);
        return to <= from ? string.Empty : value.Substring(from, to - from);
    }

    private static bool IsPathSeparator(char c) => c == '/' || c == '\\';

    private static bool IsPosixPathSeparator(char c) => c == '/';

    private static bool IsWindowsDeviceRoot(char c) => (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z');

    private static bool IsWindowsReservedName(string path, int colonIndex)
    {
        var devicePart = Slice(path, 0, colonIndex).ToUpperInvariant();
        return Array.IndexOf(WindowsReservedNames, devicePart) >= 0;
    }

    private static string NormalizeString(string path, bool allowAboveRoot, char separator, Func<char, bool> isPathSeparator)
    {
        var res = string.Empty;
        var lastSegmentLength = 0;
        var lastSlash = -1;
        var dots = 0;
        var code = '\0';
        for (var i = 0; i <= path.Length; ++i)
        {
            if (i < path.Length)
            {
                code = path[i];
            }
            else if (isPathSeparator(code))
            {
                break;
            }
            else
            {
                code = '/';
            }

            if (isPathSeparator(code))
            {
                if (lastSlash == i - 1 || dots == 1)
                {
                    // NOOP
                }
                else if (dots == 2)
                {
                    if (res.Length < 2 || lastSegmentLength != 2 || res[res.Length - 1] != '.' || res[res.Length - 2] != '.')
                    {
                        if (res.Length > 2)
                        {
                            var lastSlashIndex = res.Length - lastSegmentLength - 1;
                            if (lastSlashIndex == -1)
                            {
                                res = string.Empty;
                                lastSegmentLength = 0;
                            }
                            else
                            {
                                res = Slice(res, 0, lastSlashIndex);
                                lastSegmentLength = res.Length - 1 - res.LastIndexOf(separator);
                            }

                            lastSlash = i;
                            dots = 0;
                            continue;
                        }
                        else if (res.Length != 0)
                        {
                            res = string.Empty;
                            lastSegmentLength = 0;
                            lastSlash = i;
                            dots = 0;
                            continue;
                        }
                    }

                    if (allowAboveRoot)
                    {
                        res += res.Length > 0 ? $"{separator}.." : "..";
                        lastSegmentLength = 2;
                    }
                }
                else
                {
                    if (res.Length > 0)
                    {
                        res += separator + Slice(path, lastSlash + 1, i);
                    }
                    else
                    {
                        res = Slice(path, lastSlash + 1, i);
                    }

                    lastSegmentLength = i - lastSlash - 1;
                }

                lastSlash = i;
                dots = 0;
            }
            else if (code == '.' && dots != -1)
            {
                ++dots;
            }
            else
            {
                dots = -1;
            }
        }

        return res;
    }

    /// <summary>Node <c>path.win32.normalize</c>.</summary>
    public static string Win32Normalize(string path)
    {
        var len = path.Length;
        if (len == 0)
        {
            return ".";
        }

        var rootEnd = 0;
        string? device = null;
        var isAbsolute = false;
        var code = path[0];

        if (len == 1)
        {
            return IsPosixPathSeparator(code) ? "\\" : path;
        }

        if (IsPathSeparator(code))
        {
            isAbsolute = true;
            if (IsPathSeparator(path[1]))
            {
                var j = 2;
                var last = j;
                while (j < len && !IsPathSeparator(path[j]))
                {
                    j++;
                }

                if (j < len && j != last)
                {
                    var firstPart = Slice(path, last, j);
                    last = j;
                    while (j < len && IsPathSeparator(path[j]))
                    {
                        j++;
                    }

                    if (j < len && j != last)
                    {
                        last = j;
                        while (j < len && !IsPathSeparator(path[j]))
                        {
                            j++;
                        }

                        if (j == len || j != last)
                        {
                            if (firstPart == "." || firstPart == "?")
                            {
                                device = $"\\\\{firstPart}";
                                rootEnd = 4;
                                var colon = path.IndexOf(':');
                                var possibleDevice = Slice(path, 4, colon + 1);
                                if (IsWindowsReservedName(possibleDevice, possibleDevice.Length - 1))
                                {
                                    device = $"\\\\?\\{possibleDevice}";
                                    rootEnd = 4 + possibleDevice.Length;
                                }
                            }
                            else if (j == len)
                            {
                                return $"\\\\{firstPart}\\{Slice(path, last)}\\";
                            }
                            else
                            {
                                device = $"\\\\{firstPart}\\{Slice(path, last, j)}";
                                rootEnd = j;
                            }
                        }
                    }
                }
            }
            else
            {
                rootEnd = 1;
            }
        }
        else
        {
            var colon = path.IndexOf(':');
            if (colon > 0)
            {
                if (IsWindowsDeviceRoot(code) && colon == 1)
                {
                    device = Slice(path, 0, 2);
                    rootEnd = 2;
                    if (len > 2 && IsPathSeparator(path[2]))
                    {
                        isAbsolute = true;
                        rootEnd = 3;
                    }
                }
                else if (IsWindowsReservedName(path, colon))
                {
                    device = Slice(path, 0, colon + 1);
                    rootEnd = colon + 1;
                }
            }
        }

        var tail = rootEnd < len ? NormalizeString(Slice(path, rootEnd), !isAbsolute, '\\', IsPathSeparator) : string.Empty;
        if (tail.Length == 0 && !isAbsolute)
        {
            tail = ".";
        }

        if (tail.Length > 0 && IsPathSeparator(path[len - 1]))
        {
            tail += "\\";
        }

        if (!isAbsolute && device is null && path.Contains(':'))
        {
            if (tail.Length >= 2 && IsWindowsDeviceRoot(tail[0]) && tail[1] == ':')
            {
                return $".\\{tail}";
            }

            var index = path.IndexOf(':');
            do
            {
                if (index == len - 1 || IsPathSeparator(path[index + 1]))
                {
                    return $".\\{tail}";
                }
            }
            while ((index = path.IndexOf(':', index + 1)) != -1);
        }

        var colonIndex = path.IndexOf(':');
        if (IsWindowsReservedName(path, colonIndex))
        {
            return $".\\{device ?? string.Empty}{tail}";
        }

        if (device is null)
        {
            return isAbsolute ? $"\\{tail}" : tail;
        }

        return isAbsolute ? $"{device}\\{tail}" : $"{device}{tail}";
    }

    /// <summary>Node <c>path.win32.isAbsolute</c>.</summary>
    public static bool Win32IsAbsolute(string path)
    {
        var len = path.Length;
        if (len == 0)
        {
            return false;
        }

        var code = path[0];
        return IsPathSeparator(code) || (len > 2 && IsWindowsDeviceRoot(code) && path[1] == ':' && IsPathSeparator(path[2]));
    }

    /// <summary>Node <c>path.win32.parse(path).root</c>.</summary>
    public static string Win32Root(string path)
    {
        if (path.Length == 0)
        {
            return string.Empty;
        }

        var len = path.Length;
        var rootEnd = 0;
        var code = path[0];
        if (len == 1)
        {
            return IsPathSeparator(code) ? path : string.Empty;
        }

        if (IsPathSeparator(code))
        {
            rootEnd = 1;
            if (IsPathSeparator(path[1]))
            {
                var j = 2;
                var last = j;
                while (j < len && !IsPathSeparator(path[j]))
                {
                    j++;
                }

                if (j < len && j != last)
                {
                    last = j;
                    while (j < len && IsPathSeparator(path[j]))
                    {
                        j++;
                    }

                    if (j < len && j != last)
                    {
                        last = j;
                        while (j < len && !IsPathSeparator(path[j]))
                        {
                            j++;
                        }

                        if (j == len)
                        {
                            rootEnd = j;
                        }
                        else if (j != last)
                        {
                            rootEnd = j + 1;
                        }
                    }
                }
            }
        }
        else if (IsWindowsDeviceRoot(code) && path[1] == ':')
        {
            if (len <= 2)
            {
                return path;
            }

            rootEnd = 2;
            if (IsPathSeparator(path[2]))
            {
                if (len == 3)
                {
                    return path;
                }

                rootEnd = 3;
            }
        }

        return rootEnd > 0 ? Slice(path, 0, rootEnd) : string.Empty;
    }

    /// <summary>Node <c>path.posix.normalize</c>.</summary>
    public static string PosixNormalize(string path)
    {
        if (path.Length == 0)
        {
            return ".";
        }

        var isAbsolute = path[0] == '/';
        var trailingSeparator = path[path.Length - 1] == '/';
        path = NormalizeString(path, !isAbsolute, '/', IsPosixPathSeparator);
        if (path.Length == 0)
        {
            if (isAbsolute)
            {
                return "/";
            }

            return trailingSeparator ? "./" : ".";
        }

        if (trailingSeparator)
        {
            path += "/";
        }

        return isAbsolute ? $"/{path}" : path;
    }

    /// <summary>Node <c>path.posix.isAbsolute</c>.</summary>
    public static bool PosixIsAbsolute(string path) => path.Length > 0 && path[0] == '/';

    /// <summary>Node <c>path.posix.parse(path).root</c>.</summary>
    public static string PosixRoot(string path) => path.Length > 0 && path[0] == '/' ? "/" : string.Empty;

    /// <summary>The one casing rule: Windows and macOS fold case, Linux is exact.</summary>
    public static bool FoldsCase(string platform) => platform != CatalogPlatforms.Linux;

    /// <summary>Applies <see cref="FoldsCase"/> to one value.</summary>
    public static string CaseKey(string value, string platform) => FoldsCase(platform) ? JsString.ToLower(value) : value;

    /// <summary>Catalog <c>normalizePath</c>: normalize, then drop trailing separators unless only the root remains.</summary>
    public static string NormalizePath(string value, string platform)
    {
        var windows = platform == CatalogPlatforms.Windows;
        var normalized = windows ? Win32Normalize(value) : PosixNormalize(value);
        var root = windows ? Win32Root(normalized) : PosixRoot(normalized);
        return normalized.Length > root.Length ? TrailingSeparators.Replace(normalized, string.Empty) : normalized;
    }

    /// <summary>Catalog <c>isAbsolutePath</c>.</summary>
    public static bool IsAbsolutePath(string value, string platform) =>
        platform == CatalogPlatforms.Windows ? Win32IsAbsolute(value) : PosixIsAbsolute(value);

    /// <summary>Catalog <c>pathKeySegments</c>.</summary>
    public static List<string> PathKeySegments(string value, string platform)
    {
        var separators = platform == CatalogPlatforms.Windows ? WindowsSeparators : PosixSeparators;
        var raw = separators.Split(value);
        var segments = new List<string>();
        for (var index = 0; index < raw.Length; index++)
        {
            var segment = raw[index];
            if (segment != "." && (segment.Length != 0 || index == 0))
            {
                segments.Add(segment);
            }
        }

        if (segments.Count > 1 && segments[segments.Count - 1].Length == 0)
        {
            segments.RemoveAt(segments.Count - 1);
        }

        return segments.Select(segment => CaseKey(segment, platform)).ToList();
    }

    /// <summary>Joins key segments into one comparison key (TypeScript joins with U+0000).</summary>
    public static string JoinKey(IEnumerable<string> segments)
    {
        var builder = new StringBuilder();
        var first = true;
        foreach (var segment in segments)
        {
            if (!first)
            {
                builder.Append('\0');
            }

            builder.Append(segment);
            first = false;
        }

        return builder.ToString();
    }
}
