// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Command-line harness over the public library API, output-equivalent to the
// TypeScript `policy-catalog` CLI (src/cli.ts):
//
//   policy-catalog resolve [--catalog DIR] [--diagnostics] [--platform P]
//       [--architecture A] [--revision R] [--project-root PATH]
//       [--symbol name=value]... [--allow-weak]
//       [--purl URL] [--detected-version V] <tool>...
//   policy-catalog inspect [--catalog DIR]
//   policy-catalog validate [--catalog DIR] [--base-ref REF]
//
// Exit codes: 0 success (including "no policy"), 1 library failure or failed
// validation, 2 usage error. Output is JSON (2-space indent) on stdout.
using System.Text;
using Microsoft.Mxc.PolicyCatalog;

namespace Microsoft.Mxc.PolicyCatalog.Cli;

internal sealed class UsageException : Exception
{
    public UsageException(string message)
        : base(message)
    {
    }
}

internal static class Program
{
    private const string Usage = "usage: policy-catalog <resolve|inspect|validate> [options]";

    private static readonly Encoding Utf8 = new UTF8Encoding(false);

    public static int Main(string[] argv)
    {
        try
        {
            return Run(argv);
        }
        catch (Exception error)
        {
            Write(Console.OpenStandardError(), $"{error}\n");
            return 1;
        }
    }

    private static void Write(Stream stream, string text)
    {
        var bytes = Utf8.GetBytes(text);
        stream.Write(bytes, 0, bytes.Length);
        stream.Flush();
    }

    private static void Print(string json)
    {
        using var stdout = Console.OpenStandardOutput();
        Write(stdout, json + "\n");
    }

    private static string TakeValue(IReadOnlyList<string> args, int index, string flag)
    {
        if (index + 1 >= args.Count || args[index + 1].StartsWith("--", StringComparison.Ordinal))
        {
            throw new UsageException($"{flag} requires a value");
        }

        return args[index + 1];
    }

    private static CatalogStore StoreFrom(string? catalogDir) =>
        catalogDir is null ? CatalogStore.Bundled() : CatalogStore.FromDirectory(Path.GetFullPath(catalogDir));

    private static void RejectRest(List<string> args, string command)
    {
        if (args.Count > 0)
        {
            throw new UsageException($"{command}: unexpected argument '{args[0]}'");
        }
    }

    internal static int Run(string[] argv)
    {
        var command = argv.Length > 0 ? argv[0] : null;
        var rest = argv.Skip(1).ToList();
        try
        {
            string? catalogDir = null;
            string? baseRef = null;
            var args = new List<string>();
            for (var i = 0; i < rest.Count; i++)
            {
                if (rest[i] == "--catalog")
                {
                    catalogDir = TakeValue(rest, i, "--catalog");
                    i++;
                }
                else if (rest[i] == "--base-ref" && command == "validate")
                {
                    baseRef = TakeValue(rest, i, "--base-ref");
                    i++;
                }
                else
                {
                    args.Add(rest[i]);
                }
            }

            switch (command)
            {
                case "inspect":
                {
                    RejectRest(args, command);
                    var catalog = new PolicyCatalog(StoreFrom(catalogDir));
                    var info = catalog.GetCatalogInfo();
                    var entries = catalog.ListCatalogEntries();
                    Print(PolicyCatalogJson.SerializeInspect(info, entries, 2));
                    return 0;
                }

                case "validate":
                {
                    RejectRest(args, command);
                    var report = CatalogValidation.ValidateCatalogDirectory(catalogDir ?? CatalogValidation.BundledCatalogDirectory, baseRef);
                    Print(PolicyCatalogJson.Serialize(report, 2));
                    return report.Ok ? 0 : 1;
                }

                case "resolve":
                    return Resolve(args, catalogDir);
                default:
                    throw new UsageException(command is null ? "missing command" : $"unknown command '{command}'");
            }
        }
        catch (UsageException error)
        {
            using var stderr = Console.OpenStandardError();
            Write(stderr, $"policy-catalog: {error.Message}\n{Usage}\n");
            return 2;
        }
        catch (PolicyCatalogException error)
        {
            Print(PolicyCatalogJson.Serialize(error, 2));
            return 1;
        }
    }

    private static int Resolve(List<string> args, string? catalogDir)
    {
        string? platform = null;
        string? architecture = null;
        string? revision = null;
        string? projectRoot = null;
        var allowWeak = false;
        // Insertion-ordered; a repeated name keeps its first position and takes the last value.
        var symbolOrder = new List<string>();
        var symbolValues = new Dictionary<string, string?>(StringComparer.Ordinal);
        var tools = new List<ToolInput>();
        var diagnostics = false;
        string? purl = null;
        string? detectedVersion = null;
        for (var i = 0; i < args.Count; i++)
        {
            var arg = args[i];
            switch (arg)
            {
                case "--diagnostics":
                    diagnostics = true;
                    break;
                case "--allow-weak":
                    allowWeak = true;
                    break;
                case "--platform":
                    platform = TakeValue(args, i++, arg);
                    break;
                case "--architecture":
                    architecture = TakeValue(args, i++, arg);
                    break;
                case "--revision":
                    revision = TakeValue(args, i++, arg);
                    break;
                case "--project-root":
                    projectRoot = TakeValue(args, i++, arg);
                    break;
                case "--symbol":
                {
                    var pair = TakeValue(args, i++, arg);
                    var eq = pair.IndexOf('=', StringComparison.Ordinal);
                    if (eq <= 0)
                    {
                        throw new UsageException("--symbol expects name=value");
                    }

                    var name = pair.Substring(0, eq);
                    if (!symbolValues.ContainsKey(name))
                    {
                        symbolOrder.Add(name);
                    }

                    symbolValues[name] = pair.Substring(eq + 1);
                    break;
                }

                case "--purl":
                    purl = TakeValue(args, i++, arg);
                    break;
                case "--detected-version":
                    detectedVersion = TakeValue(args, i++, arg);
                    break;
                default:
                    if (arg.StartsWith("--", StringComparison.Ordinal))
                    {
                        throw new UsageException($"unknown option '{arg}'");
                    }

                    // --purl / --detected-version apply to the next tool name only.
                    tools.Add(new ToolInput(arg) { PackageUrl = purl, DetectedVersion = detectedVersion });
                    purl = null;
                    detectedVersion = null;
                    break;
            }
        }

        if (purl is not null || detectedVersion is not null)
        {
            throw new UsageException("--purl/--detected-version must precede a tool name");
        }

        var context = new ResolveContext
        {
            Platform = platform,
            Architecture = architecture,
            CatalogRevision = revision,
            ProjectRoot = projectRoot,
            AllowWeakIdentityFallback = allowWeak,
            Symbols = symbolOrder.Count > 0 ? new OrderedSymbols(symbolOrder, symbolValues) : null,
        };
        var catalog = new PolicyCatalog(StoreFrom(catalogDir));
        if (diagnostics)
        {
            Print(PolicyCatalogJson.Serialize(catalog.GetSandboxConfigWithDiagnostics(tools, context), 2));
        }
        else
        {
            // `null` is JSON's rendering of "no policy"; it is never an empty policy.
            Print(PolicyCatalogJson.Serialize(catalog.GetSandboxConfig(tools, context), 2));
        }

        return 0;
    }

    /// <summary>A read-only dictionary that enumerates in insertion order.</summary>
    private sealed class OrderedSymbols : IReadOnlyDictionary<string, string?>
    {
        private readonly List<string> _order;
        private readonly Dictionary<string, string?> _values;

        public OrderedSymbols(List<string> order, Dictionary<string, string?> values)
        {
            _order = order;
            _values = values;
        }

        public string? this[string key] => _values[key];

        public IEnumerable<string> Keys => _order;

        public IEnumerable<string?> Values => _order.Select(key => _values[key]);

        public int Count => _order.Count;

        public bool ContainsKey(string key) => _values.ContainsKey(key);

        public bool TryGetValue(string key, out string? value) => _values.TryGetValue(key, out value);

        public IEnumerator<KeyValuePair<string, string?>> GetEnumerator() =>
            _order.Select(key => new KeyValuePair<string, string?>(key, _values[key])).GetEnumerator();

        System.Collections.IEnumerator System.Collections.IEnumerable.GetEnumerator() => GetEnumerator();
    }
}
