/**
 * useFileUpload - File upload state and handlers
 *
 * @fileoverview Extracted from DocumentManager (OODA-13)
 * WHY: SRP - Upload orchestration is a distinct responsibility
 *
 * Phase 143: Retargeted to presigned S3 flow (D-01 / S3-as-record).
 * Upload sequence: size-check → SHA-256 → presign → direct S3 PUT echoing upload_headers.
 * No confirm endpoint (D-01). No legacy uploadDocument/uploadFile calls from this hook.
 *
 * @module edgequake_webui/hooks/use-file-upload
 */
"use client";

import type { UploadingFile } from "@/components/documents/types";
import {
  computeSha256,
  presignUpload,
  putToS3,
  type DocumentsListResult,
} from "@/lib/api/edgequake";
import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

/**
 * Maximum file size in bytes (50 MB).
 * Size is checked BEFORE hashing to avoid loading oversized files into memory (T-143-19).
 */
const MAX_FILE_SIZE_BYTES = 50 * 1024 * 1024;

export interface UseFileUploadOptions {
  /** Tenant ID for multi-tenancy */
  tenantId?: string | null;
  /** Workspace ID for isolation */
  workspaceId?: string | null;
  /**
   * Namespace for the presigned S3 upload key (D-08: taken from the current
   * workspace/documents context, NOT a new picker). If absent at upload time,
   * uploads are blocked with a clear error — no silent "default" fallback (D-08).
   */
  namespace?: string | null;
  /** Callback when upload starts (e.g., to switch filter) */
  onUploadStart?: () => void;
}

export interface UseFileUploadReturn {
  /** Files currently being uploaded with progress */
  uploadingFiles: UploadingFile[];
  /** Whether any upload is in progress */
  isUploading: boolean;
  /** Upload files handler */
  handleFilesUpload: (files: File[]) => Promise<void>;
  /** Remove a file from upload list */
  removeUploadingFile: (index: number) => void;
  /** Mark upload as complete (for PdfUploadProgress) */
  handleUploadComplete: (index: number) => void;
  /** Mark upload as failed (for PdfUploadProgress) */
  handleUploadFailed: (index: number, error: string) => void;
}

/**
 * useFileUpload - Manages file upload state and orchestration
 *
 * Phase 143 presigned-S3 flow:
 * 1. Size-check BEFORE hashing (T-143-19 DoS guard — never load oversized into memory)
 * 2. Require explicit namespace — missing namespace blocks upload, no silent fallback (D-08)
 * 3. SHA-256 via native Web Crypto (no npm dep)
 * 4. POST /documents/presign → duplicate short-circuit (D-03) or upload_url + upload_headers
 * 5. PUT file directly to S3 echoing ONLY the signed upload_headers (T-143-11/T-143-18)
 * 6. Invalidate BOTH documents + raw-docs query keys for immediate list refresh
 *
 * No confirm endpoint (D-01). No legacy uploadDocument/uploadFile calls.
 * PDF path retargeted to the same presign flow (D-06: content-type agnostic presign).
 */
export function useFileUpload(
  options: UseFileUploadOptions = {},
): UseFileUploadReturn {
  const { tenantId, workspaceId, namespace, onUploadStart } = options;

  const [uploadingFiles, setUploadingFiles] = useState<UploadingFile[]>([]);
  const [isUploading, setIsUploading] = useState(false);

  const queryClient = useQueryClient();
  const router = useRouter();
  const { t } = useTranslation();

  /**
   * Main upload handler — presigned S3 flow (Phase 143).
   * Processes files sequentially for per-file feedback and error isolation.
   */
  const handleFilesUpload = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;

      // Notify parent (e.g., to switch status filter)
      onUploadStart?.();

      setIsUploading(true);

      // Initialize upload state for all files
      const initialFiles: UploadingFile[] = files.map((file) => ({
        file,
        progress: 0,
        status: "pending" as const,
        phase: "Waiting...",
      }));
      setUploadingFiles(initialFiles);

      // Show loading toast
      const toastId = toast.loading(
        t("documents.upload.inProgress", { count: files.length }) ||
          `Uploading ${files.length} file(s)...`,
        { duration: Infinity },
      );

      let successCount = 0;
      let errorCount = 0;

      // Process files sequentially for better feedback
      for (let i = 0; i < files.length; i++) {
        const file = files[i];

        try {
          // ── Step 1: Size check BEFORE hashing (T-143-19: never arrayBuffer an oversized file) ──
          if (file.size > MAX_FILE_SIZE_BYTES) {
            const sizeMb = (file.size / 1024 / 1024).toFixed(1);
            toast.error(
              `${file.name} is too large (${sizeMb} MB). Maximum allowed: ${MAX_FILE_SIZE_BYTES / 1024 / 1024} MB.`,
              { duration: 6000 },
            );
            setUploadingFiles((prev) =>
              prev.map((f, idx) =>
                idx === i
                  ? {
                      ...f,
                      status: "error" as const,
                      progress: 100,
                      error: `File too large (${sizeMb} MB)`,
                      phase: t("common.failed", "Failed"),
                    }
                  : f,
              ),
            );
            errorCount++;
            continue;
          }

          // ── Step 2: Require explicit namespace — no silent fallback (D-08) ──
          if (!namespace) {
            toast.error(
              `No namespace selected for this workspace — cannot upload ${file.name}. Please select a namespace context before uploading.`,
              { duration: 6000 },
            );
            setUploadingFiles((prev) =>
              prev.map((f, idx) =>
                idx === i
                  ? {
                      ...f,
                      status: "error" as const,
                      progress: 100,
                      error: "No namespace selected for this workspace",
                      phase: t("common.failed", "Failed"),
                    }
                  : f,
              ),
            );
            errorCount++;
            continue;
          }

          // Phase: Reading file (SHA-256)
          setUploadingFiles((prev) =>
            prev.map((f, idx) =>
              idx === i
                ? {
                    ...f,
                    status: "reading" as const,
                    progress: 10,
                    phase: t("documents.upload.reading", "Reading file..."),
                  }
                : f,
            ),
          );

          // ── Step 3: Compute SHA-256 (native Web Crypto, no npm dep) ──
          const sha256 = await computeSha256(file);

          // Phase: Presigning
          setUploadingFiles((prev) =>
            prev.map((f, idx) =>
              idx === i
                ? {
                    ...f,
                    status: "uploading" as const,
                    progress: 30,
                    phase: t(
                      "documents.upload.uploading",
                      "Uploading to server...",
                    ),
                  }
                : f,
            ),
          );

          // ── Step 4: POST /documents/presign ──
          const presign = await presignUpload({
            filename: file.name,
            size: file.size,
            sha256,
            content_type: file.type || undefined,
            namespace,
          });

          // ── Step 5: Duplicate short-circuit (D-03) ──
          if (presign.is_duplicate) {
            toast.warning(
              t(
                "documents.upload.duplicate",
                "{{name}} is a duplicate (already uploaded)",
                { name: file.name },
              ),
              { duration: 4000 },
            );
            setUploadingFiles((prev) =>
              prev.map((f, idx) =>
                idx === i
                  ? {
                      ...f,
                      status: "success" as const,
                      progress: 100,
                      phase: t(
                        "documents.upload.duplicateSkipped",
                        "Duplicate (skipped)",
                      ),
                    }
                  : f,
              ),
            );
            successCount++;
            continue;
          }

          // ── Step 6: PUT directly to S3, echoing signed upload_headers (T-143-11/T-143-18) ──
          // putToS3 uses raw fetch — NOT the api client. Only the presign upload_headers
          // are attached (signed x-amz-meta-* + content-type). Adding Authorization/tenant
          // headers here would break the SigV4 signature and return 403.
          if (!presign.upload_url) {
            throw new Error(
              "Presign response missing upload_url (not a duplicate)",
            );
          }

          setUploadingFiles((prev) =>
            prev.map((f, idx) =>
              idx === i
                ? {
                    ...f,
                    progress: 60,
                    phase: "Uploading to S3...",
                  }
                : f,
            ),
          );

          await putToS3(presign.upload_url, file, presign.upload_headers);

          // ── Step 7: Mark complete ──
          setUploadingFiles((prev) =>
            prev.map((f, idx) =>
              idx === i
                ? {
                    ...f,
                    status: "success" as const,
                    progress: 100,
                    phase: t("documents.upload.complete", "Complete!"),
                  }
                : f,
            ),
          );

          successCount++;
        } catch (error) {
          const errorMessage =
            error instanceof Error ? error.message : "Upload failed";
          setUploadingFiles((prev) =>
            prev.map((f, idx) =>
              idx === i
                ? {
                    ...f,
                    status: "error" as const,
                    progress: 100,
                    error: errorMessage,
                    phase: t("common.failed", "Failed"),
                  }
                : f,
            ),
          );

          errorCount++;
        }
      }

      // Update toast with final result
      if (errorCount === 0) {
        toast.success(
          t("documents.upload.success", { count: successCount }) ||
            `Successfully uploaded ${successCount} file(s)`,
          {
            id: toastId,
            duration: 5000,
            action: {
              label: t("documents.upload.viewInGraph", "View in Graph"),
              onClick: () => router.push("/graph"),
            },
          },
        );
      } else if (successCount === 0) {
        toast.error(
          t("documents.upload.allFailed", { count: errorCount }) ||
            `All ${errorCount} file(s) failed to upload`,
          {
            id: toastId,
            duration: 5000,
            action: {
              label: t("common.retry", "Retry"),
              onClick: () => {
                setUploadingFiles([]);
              },
            },
          },
        );
      } else {
        toast.warning(
          t("documents.upload.partial", {
            success: successCount,
            failed: errorCount,
          }) || `Uploaded ${successCount} file(s), ${errorCount} failed`,
          {
            id: toastId,
            duration: 5000,
            action: {
              label: t("documents.upload.viewInGraph", "View in Graph"),
              onClick: () => router.push("/graph"),
            },
          },
        );
      }

      // Refresh both documents list AND raw-docs list.
      // WHY: invalidate "documents" for existing consumers + "raw-docs" for the
      // new S3-listed raw-docs list (Task 4). Both query keys use the same
      // invalidation so an upload immediately surfaces in the list.
      await queryClient.invalidateQueries({ queryKey: ["documents"] });
      await queryClient.invalidateQueries({ queryKey: ["raw-docs"] });
      queryClient.refetchQueries({
        queryKey: ["documents"],
        type: "active",
      });
      queryClient.refetchQueries({
        queryKey: ["raw-docs"],
        type: "active",
      });

      setIsUploading(false);

      // Clear upload list after delay
      setTimeout(() => {
        setUploadingFiles([]);
      }, 3000);
    },
    [queryClient, t, router, namespace, tenantId, workspaceId, onUploadStart],
  );

  /**
   * Remove a file from the upload list
   */
  const removeUploadingFile = useCallback((index: number) => {
    setUploadingFiles((prev) => prev.filter((_, i) => i !== index));
  }, []);

  /**
   * Mark PDF upload as successful (called by PdfUploadProgress)
   */
  const handleUploadComplete = useCallback((index: number) => {
    setUploadingFiles((prev) =>
      prev.map((f, idx) =>
        idx === index ? { ...f, status: "success" as const, progress: 100 } : f,
      ),
    );
  }, []);

  /**
   * Mark PDF upload as failed (called by PdfUploadProgress)
   */
  const handleUploadFailed = useCallback((index: number, error: string) => {
    setUploadingFiles((prev) =>
      prev.map((f, idx) =>
        idx === index ? { ...f, status: "error" as const, error } : f,
      ),
    );
  }, []);

  return {
    uploadingFiles,
    isUploading,
    handleFilesUpload,
    removeUploadingFile,
    handleUploadComplete,
    handleUploadFailed,
  };
}

export default useFileUpload;
