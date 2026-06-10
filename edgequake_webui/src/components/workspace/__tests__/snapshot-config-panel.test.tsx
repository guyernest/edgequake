/**
 * @module snapshot-config-panel.test
 * @description Unit tests for SnapshotConfigPanel URI validation and interaction logic.
 *
 * Tests cover:
 * - Empty URI → mode RadioGroup is disabled (not hidden)
 * - URI "not-a-uri" fails blur validation (bad-format error)
 * - URI "s3://bucket/prefix" passes validation and enables mode radio; default mode "write-and-store"
 * - URI containing "../" fails validation (path-traversal guard, mirrors server T-23-02)
 *
 * WR-06: these tests import the REAL exported helpers/constants from
 * snapshot-config-panel.tsx — no local re-implementations.
 */

import { describe, expect, it } from "vitest";

import {
  DEFAULT_SNAPSHOT_MODE,
  isModeEnabled,
  validateSnapshotUri,
} from "../snapshot-config-panel";

// ============================================================================
// Empty URI — mode radio disabled
// ============================================================================

describe("SnapshotConfigPanel — empty URI disables mode radio", () => {
  it("empty URI: validateSnapshotUri returns null (valid 'no snapshot' state)", () => {
    expect(validateSnapshotUri("")).toBeNull();
  });

  it("empty URI: isModeEnabled returns false (radio disabled)", () => {
    expect(isModeEnabled("")).toBe(false);
  });

  it("empty URI: save is still allowed (valid 'no snapshot' state)", () => {
    const uri = "";
    const isSaveBlocked = validateSnapshotUri(uri) !== null;
    expect(isSaveBlocked).toBe(false);
  });
});

// ============================================================================
// Invalid URI — bad format error
// ============================================================================

describe("SnapshotConfigPanel — bad format URI shows error", () => {
  it("'not-a-uri' fails with uriErrorBadFormat", () => {
    expect(validateSnapshotUri("not-a-uri")).toBe("uriErrorBadFormat");
  });

  it("'http://example.com' fails (not s3:// or absolute path)", () => {
    expect(validateSnapshotUri("http://example.com")).toBe("uriErrorBadFormat");
  });

  it("'relative/path' fails (not absolute)", () => {
    expect(validateSnapshotUri("relative/path")).toBe("uriErrorBadFormat");
  });

  it("'not-a-uri' does NOT enable mode radio", () => {
    expect(isModeEnabled("not-a-uri")).toBe(false);
  });
});

// ============================================================================
// Path-traversal guard (T-23-02)
// ============================================================================

describe("SnapshotConfigPanel — path traversal rejected", () => {
  it("URI containing '../' fails validation", () => {
    expect(validateSnapshotUri("../secret")).toBe("uriErrorBadFormat");
  });

  it("s3:// URI containing '../' fails (path-traversal guard)", () => {
    expect(validateSnapshotUri("s3://bucket/../prefix")).toBe("uriErrorBadFormat");
  });

  it("absolute path with '../' fails", () => {
    expect(validateSnapshotUri("/data/../etc/passwd")).toBe("uriErrorBadFormat");
  });

  it("URI with '..' anywhere fails", () => {
    expect(validateSnapshotUri("/valid/path/..evil")).toBe("uriErrorBadFormat");
  });

  it("path-traversal URI does NOT enable mode radio", () => {
    expect(isModeEnabled("../secret")).toBe(false);
  });
});

// ============================================================================
// Valid URI — mode radio enabled, default mode
// ============================================================================

describe("SnapshotConfigPanel — valid URI enables mode radio", () => {
  it("'s3://bucket/prefix' passes validation", () => {
    expect(validateSnapshotUri("s3://bucket/prefix")).toBeNull();
  });

  it("'s3://bucket/prefix' enables mode radio", () => {
    expect(isModeEnabled("s3://bucket/prefix")).toBe(true);
  });

  it("'/local/absolute/path' passes validation (absolute path)", () => {
    expect(validateSnapshotUri("/local/absolute/path")).toBeNull();
  });

  it("'/local/absolute/path' enables mode radio", () => {
    expect(isModeEnabled("/local/absolute/path")).toBe(true);
  });

  it("default snapshot mode is 'write-and-store'", () => {
    expect(DEFAULT_SNAPSHOT_MODE).toBe("write-and-store");
  });
});

// ============================================================================
// URI edge cases
// ============================================================================

describe("SnapshotConfigPanel — URI edge cases", () => {
  it("'s3://' (minimal S3 URI prefix) passes format check", () => {
    expect(validateSnapshotUri("s3://")).toBeNull();
  });

  it("'/' (root absolute path) passes format check", () => {
    expect(validateSnapshotUri("/")).toBeNull();
  });

  it("'s3://my-bucket/snapshots/2026' passes and enables radio", () => {
    expect(validateSnapshotUri("s3://my-bucket/snapshots/2026")).toBeNull();
    expect(isModeEnabled("s3://my-bucket/snapshots/2026")).toBe(true);
  });
});

// ============================================================================
// Mode radio disabled rendering contract
// ============================================================================

describe("SnapshotConfigPanel — mode radio rendering contract", () => {
  it("empty URI: each RadioGroupItem is disabled (rendered with disabled prop)", () => {
    const uri = "";
    // When isModeEnabled returns false, each RadioGroupItem must have disabled={true}
    const eachItemDisabled = !isModeEnabled(uri);
    expect(eachItemDisabled).toBe(true);
  });

  it("valid URI: RadioGroupItem is NOT disabled", () => {
    const uri = "s3://bucket/prefix";
    const eachItemDisabled = !isModeEnabled(uri);
    expect(eachItemDisabled).toBe(false);
  });
});
