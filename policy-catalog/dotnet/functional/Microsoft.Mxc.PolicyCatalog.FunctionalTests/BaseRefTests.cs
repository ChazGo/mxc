// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// validate --base-ref (published-revision immutability) through the packaged CLI.
using System.Diagnostics;
using System.Text.Json.Nodes;
using Xunit;
using static Microsoft.Mxc.PolicyCatalog.FunctionalTests.Fx;

namespace Microsoft.Mxc.PolicyCatalog.FunctionalTests;

public sealed class BaseRefTests : WorkDirTest
{
    private static string Git(string cwd, params string[] args)
    {
        var start = new ProcessStartInfo("git") { WorkingDirectory = cwd, RedirectStandardOutput = true, RedirectStandardError = true, UseShellExecute = false };
        foreach (var arg in new[] { "-c", "user.name=functional-test", "-c", "user.email=functional-test@invalid", "-c", "core.autocrlf=false", "-c", "commit.gpgsign=false" }.Concat(args))
        {
            start.ArgumentList.Add(arg);
        }

        using var process = Process.Start(start)!;
        var stderr = process.StandardError.ReadToEndAsync();
        var output = process.StandardOutput.ReadToEnd();
        process.WaitForExit();
        Assert.True(process.ExitCode == 0, $"git {string.Join(" ", args)}: {stderr.Result}");
        return output;
    }

    private (string Repo, string Dir) PublishedRepo()
    {
        var repo = NewDir("repo-");
        Git(repo, "init", "-q");
        var dir = Path.Combine(repo, "catalog");
        WriteCatalog(dir, new[] { Revision(new JsonNode[] { Entry("tool:a") }, "2000-01-01.1") });
        Git(repo, "add", "-A");
        Git(repo, "commit", "-q", "-m", "publish");
        return (repo, dir);
    }

    [Fact]
    public void UnchangedOrAppendedPasses()
    {
        var (_, dir) = PublishedRepo();
        var unchanged = Cli("validate", "--catalog", dir, "--base-ref", "HEAD");
        Assert.Equal(0, unchanged.Status);
        Assert.Equal("""{"ref":"HEAD","comparedRevisions":1}""", unchanged.Json!["baseRef"]!.ToJsonString(Relaxed));
        WriteCatalog(dir, new[]
        {
            Revision(new JsonNode[] { Entry("tool:a") }, "2000-01-01.1"),
            Revision(new JsonNode[] { Entry("tool:a"), Entry("tool:b") }, "2000-01-02.1"),
        });
        Assert.Equal(0, Cli("validate", "--catalog", dir, "--base-ref", "HEAD").Status);
    }

    [Fact]
    public void RewritingAPublishedRevisionFails()
    {
        var (_, dir) = PublishedRepo();
        WriteCatalog(dir, new[] { Revision(new JsonNode[] { Entry("tool:a", """{ "entryRevision": 2, "displayName": "changed" }""") }, "2000-01-01.1") });
        Assert.Equal(0, Cli("validate", "--catalog", dir).Status);
        var result = Cli("validate", "--catalog", dir, "--base-ref", "HEAD");
        Assert.Equal(1, result.Status);
        Assert.Equal(
            """["[immutability] published revision '2000-01-01.1' manifest entry was modified","[immutability] published revision file 'revisions/2000-01-01.1.json' was modified; publish a new revision instead"]""",
            result.Json!["errors"]!.ToJsonString(Relaxed));
    }

    [Fact]
    public void BadRefsAndNonGitDirectoriesAreErrors()
    {
        var (repo, dir) = PublishedRepo();
        Assert.Contains("base-ref check: 'no-such-ref' does not name a commit", Cli("validate", "--catalog", dir, "--base-ref", "no-such-ref").Stdout, StringComparison.Ordinal);
        var optionLike = Cli("validate", "--catalog", dir, "--base-ref", "-h");
        Assert.Equal(1, optionLike.Status);
        Assert.Contains("base-ref check: '-h' is not a valid git ref", optionLike.Stdout, StringComparison.Ordinal);
        var outside = WriteCatalog(Path.Combine(Path.GetTempPath(), "policy-catalog-dotnet-nogit-" + Guid.NewGuid().ToString("N")), new[] { Revision(new JsonNode[] { Entry("tool:a") }) });
        try
        {
            var noGit = Cli("validate", "--catalog", outside, "--base-ref", "HEAD");
            Assert.Equal(1, noGit.Status);
            Assert.Contains("is not inside a git work tree", noGit.Stdout, StringComparison.Ordinal);
        }
        finally
        {
            Directory.Delete(outside, true);
        }

        var emptyTree = Git(repo, "mktree").Trim();
        var orphan = Git(repo, "commit-tree", emptyTree, "-m", "empty").Trim();
        var empty = Cli("validate", "--catalog", dir, "--base-ref", orphan);
        Assert.Equal(0, empty.Status);
        Assert.Equal(0, empty.Json!["baseRef"]!["comparedRevisions"]!.GetValue<int>());
    }
}
