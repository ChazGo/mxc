// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

using System.Text.Json.Nodes;
using Microsoft.Mxc.PolicyCatalog.Internal;
using Xunit;

namespace Microsoft.Mxc.PolicyCatalog.Tests;

/// <summary>Port of tests/unit/store.test.ts: integrity, revisions, history, immutability.</summary>
public sealed class StoreTests
{
    private static readonly CatalogContract Contract = Catalog.ValidateContract(JsonText.Parse(TestData.Contract));

    private static CatalogRevisionData Validated(JsonNode revision) => Catalog.ValidateRevision(JsonText.Parse(revision.ToJsonString()), Contract);

    [Fact]
    public void BundledCatalogVerifies()
    {
        var store = CatalogStore.Bundled();
        Assert.Empty(CatalogValidation.CheckStoreHistory(store));
        Assert.Equal("2026-09-29.1", store.DefaultRevision);
        store.Verify();
    }

    [Fact]
    public void CanonicalDigestIgnoresFormattingButNotContent()
    {
        Assert.Equal("""{"a":[2,{"c":4,"d":3}],"b":1}""", PolicyCatalogJson.Canonicalize("""{ "b": 1, "a": [2, { "d": 3, "c": 4 }] }"""));
        Assert.Equal(PolicyCatalogJson.CanonicalSha256("""{"a":1,"b":2}"""), PolicyCatalogJson.CanonicalSha256("{\r\n \"b\": 2,\n\"a\": 1}"));
        Assert.NotEqual(PolicyCatalogJson.CanonicalSha256("""{"a":1}"""), PolicyCatalogJson.CanonicalSha256("""{"a":2}"""));
    }

    [Fact]
    public void TamperedRevisionIsAnIntegrityError()
    {
        var store = TestData.StoreFor(new[] { TestData.Revision(new[] { TestData.Entry("tool:a") }) }, digests: new Dictionary<string, string> { ["2000-01-01.1"] = new string('0', 64) });
        var error = TestData.Failure(() => store.Verify());
        Assert.Equal(("backend_error", "integrity"), (error.Code, error.Reason));
        Assert.Matches("^\\[backend_error\\] catalog revision '2000-01-01.1' digest [0-9a-f]{64} does not match the published digest 0{64}$", error.Message);
        var catalog = new PolicyCatalog(store, new FixedHost());
        var weak = new ResolveContext { AllowWeakIdentityFallback = true };
        Assert.Equal("integrity", TestData.ErrorReason(() => catalog.ResolveSandboxPolicy("a", weak)));
        Assert.Equal("integrity", TestData.ErrorReason(() => catalog.ResolveSandboxPolicyWithDiagnostics("a", weak)));
        Assert.Equal("integrity", TestData.ErrorReason(() => catalog.ListCatalogEntries()));
        Assert.Equal("integrity", TestData.ErrorReason(() => catalog.GetCatalogInfo()));
    }

    [Fact]
    public void RevisionIdMismatchIsAnIntegrityError()
    {
        var revision = TestData.Revision(new[] { TestData.Entry("tool:a") }, "2000-01-02.1");
        var relabelled = revision.DeepClone();
        relabelled["catalogRevision"] = "2000-01-01.1";
        // The digest was computed for other content, so this is caught as tampering first.
        var store = TestData.StoreFor(new[] { relabelled }, digests: new Dictionary<string, string> { ["2000-01-01.1"] = PolicyCatalogJson.CanonicalSha256(revision.ToJsonString()) });
        Assert.Equal("integrity", TestData.ErrorReason(() => store.Verify()));

        // Correct digest, but the file declares another revision.
        var files = new Dictionary<string, string> { ["revisions/2000-01-01.1.json"] = revision.ToJsonString() };
        var manifest = $$"""{"catalogSchemaVersion":"1","defaultRevision":"2000-01-01.1","revisions":[{"catalogRevision":"2000-01-01.1","file":"revisions/2000-01-01.1.json","sha256":"{{PolicyCatalogJson.CanonicalSha256(revision.ToJsonString())}}"}]}""";
        var mismatched = CatalogStore.FromJson(TestData.Contract, manifest, file => files[file]);
        var error = TestData.Failure(() => mismatched.Verify());
        Assert.Equal("[backend_error] file 'revisions/2000-01-01.1.json' declares revision '2000-01-02.1', expected '2000-01-01.1'", error.Message);
    }

    [Fact]
    public void InvalidManifestIsAValidationError()
    {
        var revision = TestData.Revision(new[] { TestData.Entry("tool:a") });
        Assert.Equal("invalid_catalog", TestData.ErrorReason(() => TestData.StoreFor(new[] { revision }, defaultRevision: "2001-01-01.1")));
        var error = TestData.Failure(() => TestData.StoreFor(new[] { TestData.Revision(new[] { TestData.Entry("tool:a") }, "2000-01-02.1"), revision }));
        Assert.Equal("[policy_validation] manifest.revisions must be strictly increasing ('2000-01-01.1')", error.Message);
    }

    [Fact]
    public void ExplicitRevisionIsNeverSubstituted()
    {
        var r1 = TestData.Revision(new[] { TestData.Entry("tool:a"), TestData.Entry("tool:b") }, "2000-01-01.1");
        var r2 = TestData.Revision(new[] { TestData.Entry("tool:a", """{ "entryRevision": 2, "displayName": "renamed" }"""), TestData.Entry("tool:b") }, "2000-01-02.1");
        var catalog = new PolicyCatalog(TestData.StoreFor(new[] { r1, r2 }), new FixedHost());
        var ctx = new ResolveContext { AllowWeakIdentityFallback = true, ProjectRoot = "/p" };
        var latest = catalog.ResolveSandboxPolicyWithDiagnostics("a", ctx);
        Assert.Equal(("2000-01-02.1", 2.0), (latest.Diagnostics.CatalogRevision, latest.Diagnostics.Tools[0].Matches[0].EntryRevision));
        var older = catalog.ResolveSandboxPolicyWithDiagnostics("a", ctx with { CatalogRevision = "2000-01-01.1" });
        Assert.Equal(("2000-01-01.1", 1.0), (older.Diagnostics.CatalogRevision, older.Diagnostics.Tools[0].Matches[0].EntryRevision));
        var error = TestData.Failure(() => catalog.ResolveSandboxPolicy("a", ctx with { CatalogRevision = "2000-01-03.1" }));
        Assert.Equal(("backend_error", "revision_unavailable", "[backend_error] catalog revision '2000-01-03.1' is not installed"), (error.Code, error.Reason, error.Message));
    }

    [Fact]
    public void EntryRevisionMustIncreaseExactlyWhenAnEntryChanges()
    {
        var r1 = Validated(TestData.Revision(new[] { TestData.Entry("tool:a"), TestData.Entry("tool:b") }, "2000-01-01.1"));
        var changedNoBump = Validated(TestData.Revision(new[] { TestData.Entry("tool:a", """{ "displayName": "x" }"""), TestData.Entry("tool:b") }, "2000-01-02.1"));
        var bumpedNoChange = Validated(TestData.Revision(new[] { TestData.Entry("tool:a"), TestData.Entry("tool:b", """{ "entryRevision": 2 }""") }, "2000-01-02.1"));
        var ok = Validated(TestData.Revision(new[] { TestData.Entry("tool:a", """{ "displayName": "x", "entryRevision": 2 }"""), TestData.Entry("tool:b") }, "2000-01-02.1"));
        Assert.Equal(new[] { "2000-01-02.1: 'tool:a' changed but entryRevision did not increase (1 -> 1)" }, CatalogValidation.CheckEntryRevisions(r1, changedNoBump));
        Assert.Equal(new[] { "2000-01-02.1: 'tool:b' is unchanged but entryRevision moved (1 -> 2)" }, CatalogValidation.CheckEntryRevisions(r1, bumpedNoChange));
        Assert.Empty(CatalogValidation.CheckEntryRevisions(r1, ok));
        Assert.Contains("catalog revision '2000-01-01.1' must be newer than '2000-01-02.1'", CatalogValidation.CheckEntryRevisions(ok, r1));
    }

    private static CatalogValidation.PublishedRevision Rev(string id, string sha) =>
        new(new JsonString(id), new JsonString($"revisions/{id}.json"), new JsonString(sha));

    [Fact]
    public void PublishedRevisionsAreImmutable()
    {
        var published = Rev("2000-01-01.1", new string('a', 64));
        var files = new Dictionary<string, string> { ["revisions/2000-01-01.1.json"] = "{\"x\":1}\n" };
        var @base = new CatalogValidation.PublishedState(new[] { published }, files);
        Assert.Empty(CatalogValidation.CheckPublishedImmutability(@base, new(new[] { published, Rev("2000-01-02.1", new string('b', 64)) }, new Dictionary<string, string> { ["revisions/2000-01-01.1.json"] = "{ \"x\" : 1 }\r\n" })));
        Assert.Equal(
            new[] { "published revision file 'revisions/2000-01-01.1.json' was modified; publish a new revision instead" },
            CatalogValidation.CheckPublishedImmutability(@base, new(new[] { published }, new Dictionary<string, string> { ["revisions/2000-01-01.1.json"] = "{\"x\":2}\n" })));
        Assert.Equal(
            new[] { "published revision '2000-01-01.1' manifest entry was modified" },
            CatalogValidation.CheckPublishedImmutability(@base, new(new[] { Rev("2000-01-01.1", new string('c', 64)) }, files)));
        Assert.Equal(
            new[] { "published revision '2000-01-01.1' was removed or reordered in the manifest" },
            CatalogValidation.CheckPublishedImmutability(@base, new(Array.Empty<CatalogValidation.PublishedRevision>(), new Dictionary<string, string>())));
        Assert.Equal(
            new[] { "published revision file 'revisions/2000-01-01.1.json' was modified; publish a new revision instead" },
            CatalogValidation.CheckPublishedImmutability(@base, new(new[] { published }, new Dictionary<string, string> { ["revisions/2000-01-01.1.json"] = "not json" })));
    }

    [Fact]
    public void DirectoryStoreReportsUnreadableData()
    {
        var dir = Path.Combine(Path.GetTempPath(), "pc-dotnet-unit-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        try
        {
            var error = TestData.Failure(() => CatalogStore.FromDirectory(dir));
            Assert.Equal("integrity", error.Reason);
            Assert.StartsWith("[backend_error] catalog data could not be read: ", error.Message, StringComparison.Ordinal);
            var report = CatalogValidation.ValidateCatalogDirectory(dir);
            Assert.False(report.Ok);
            Assert.Null(report.DefaultRevision);
            Assert.Equal(Path.GetFullPath(dir), report.CatalogDir);
        }
        finally
        {
            Directory.Delete(dir, true);
        }
    }

    [Fact]
    public void BaseRefRejectsOptionLikeRefsWithoutRunningGit()
    {
        var (compared, errors) = CatalogValidation.CheckAgainstBaseRef(Path.GetTempPath(), Array.Empty<ManifestRevision>(), "-h");
        Assert.Equal(0, compared);
        Assert.Equal(new[] { "[policy_validation] base-ref check: '-h' is not a valid git ref" }, errors);
        Assert.Equal("[policy_validation] base-ref check: 'a b' is not a valid git ref", CatalogValidation.CheckAgainstBaseRef(Path.GetTempPath(), Array.Empty<ManifestRevision>(), "a b").Errors.Single());
    }
}
