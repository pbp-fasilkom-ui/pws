import { createFileRoute, useParams, redirect } from "@tanstack/react-router";
import useSWR from "swr";
import { useEffect, useMemo, useState, Fragment } from "react";

async function checkProjectAccess(owner: string, project: string) {
  try {
    const apiUrl = (import.meta as any).env.VITE_API_URL;
    const response = await fetch(`${apiUrl}/project/${owner}/${project}/access`, {
      credentials: "include",
      headers: {
        "Content-Type": "application/json"
      }
    });
    return response.ok;
  } catch (error) {
    console.error('Error checking project access:', error);
    return false;
  }
}

export const Route = createFileRoute("/project/$owner/$project/code")({
  beforeLoad: async ({ params }: { params: { owner: string; project: string } }) => {
    const hasAccess = await checkProjectAccess(params.owner, params.project);
    if (!hasAccess) {
      throw redirect({ to: '/', search: { error: 'access_denied' } });
    }
  },
  component: CodeBrowser,
});

const apiFetcher = (input: URL | RequestInfo, options?: RequestInit) =>
  fetch(input, {
    ...options,
    redirect: "follow",
    credentials: "include",
    headers: { "Content-Type": "application/json" },
  }).then((res) => {
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return res.json();
  });

type TreeEntry =
  | { kind: "dir"; name: string }
  | { kind: "file"; name: string; size: number }
  | { kind: "symlink"; name: string }
  | { kind: "submodule"; name: string }
  | { kind: "other"; name: string };

type TreeResponse = {
  ref: string;
  path: string;
  is_empty_repo: boolean;
  entries: TreeEntry[];
};

type RefsResponse = {
  default_branch: string | null;
  deployed_branch: string | null;
  branches: string[];
};

type FileResponse = {
  ref: string;
  path: string;
  size: number;
  content: string;
};

function CodeBrowser() {
  // @ts-ignore
  const { owner, project } = useParams({ strict: false });
  const [ref, setRef] = useState<string | null>(null);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const refsUrl = `${import.meta.env.VITE_API_URL}/project/${owner}/${project}/refs`;
  const { data: refs, isLoading: refsLoading } = useSWR<RefsResponse>(
    refsUrl,
    apiFetcher,
  );

  useEffect(() => {
    if (!refs || ref) return;
    const initialRef =
      (refs.deployed_branch && refs.branches.includes(refs.deployed_branch)
        ? refs.deployed_branch
        : null) ??
      (refs.default_branch && refs.branches.includes(refs.default_branch)
        ? refs.default_branch
        : null) ??
      refs.branches[0] ??
      null;
    setRef(initialRef);
  }, [refs, ref]);

  return (
    <div className="w-full">
      {/* Header / Controls */}
      <div className="mb-4 flex items-center gap-3">
        <span className="text-sm text-slate-300">Ref</span>
        <select
          value={ref ?? ""}
          onChange={(e) => {
            setRef(e.target.value);
            setSelectedFile(null);
          }}
          disabled={refsLoading || !ref}
          className="min-w-56 rounded-md bg-slate-800 px-3 py-2 text-sm outline-none ring-1 ring-slate-700 focus:ring-slate-500 disabled:opacity-60"
        >
          {!ref && <option value="">Loading branches...</option>}
          {refs?.branches.map((branch) => (
            <option key={branch} value={branch}>
              {branch}
              {branch === refs.deployed_branch ? " (deployed)" : ""}
              {branch === refs.default_branch ? " (default branch)" : ""}
            </option>
          ))}
        </select>
      </div>

      {!ref ? (
        <div className="rounded-lg border border-slate-700 bg-slate-900">
          <RowSkeleton label="Loading repository branches..." />
        </div>
      ) : selectedFile ? (
        <FilePreview
          owner={owner}
          project={project}
          refValue={ref}
          path={selectedFile}
          onBack={() => setSelectedFile(null)}
        />
      ) : (
        <div className="rounded-lg border border-slate-700 bg-slate-900">
          <Tree owner={owner} project={project} refValue={ref} path="" onSelectFile={setSelectedFile} />
        </div>
      )}
    </div>
  );
}

function Tree({
  owner,
  project,
  refValue,
  path,
  onSelectFile,
}: {
  owner: string;
  project: string;
  refValue: string;
  path: string;
  onSelectFile: (path: string) => void;
}) {
  const url = useMemo(() => {
    const base = import.meta.env.VITE_API_URL as string;
    const u = new URL(`${base}/project/${owner}/${project}/tree`);
    if (refValue) u.searchParams.set("ref", refValue);
    if (path) u.searchParams.set("path", path);
    return u.toString();
  }, [owner, project, refValue, path]);

  const { data, error, isLoading } = useSWR<TreeResponse>(
    ["tree", url],
    ([, u]) => apiFetcher(u),
  );

  if (isLoading) {
    return (
      <RowSkeleton
        label={path ? `Loading ${path}...` : "Loading repository..."}
      />
    );
  }
  if (error) {
    return (
      <ErrorRow message={`Failed to load tree: ${(error as Error).message}`} />
    );
  }
  if (!data) return null;

  if (data.is_empty_repo) {
    return (
      <div className="p-4 text-sm text-slate-400">
        This repository is empty. Push something to get started.
      </div>
    );
  }

  // Render the immediate children at (ref, path)
  return (
    <ul className="divide-y divide-slate-800">
      {data.entries.map((e) => (
        <Fragment key={`${path}/${e.name}`}>
          {e.kind === "dir" ? (
            <DirNode
              owner={owner}
              project={project}
              refValue={refValue}
              parentPath={path}
              name={e.name}
              onSelectFile={onSelectFile}
            />
          ) : (
            <Leaf entry={e} onClick={e.kind === "file" ? () => onSelectFile(joinPath(path, e.name)) : undefined} />
          )}
        </Fragment>
      ))}
    </ul>
  );
}

function DirNode({
  owner,
  project,
  refValue,
  parentPath,
  name,
  onSelectFile,
}: {
  owner: string;
  project: string;
  refValue: string;
  parentPath: string;
  name: string;
  onSelectFile: (path: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const childPath = joinPath(parentPath, name);

  return (
    <li className="group">
      <button
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-slate-800/60"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
      >
        <ChevronIcon open={open} />
        <FolderIcon />
        <span className="truncate">{name}</span>
      </button>

      {open && (
        <div className="pl-7">
          <Tree
            owner={owner}
            project={project}
            refValue={refValue}
            path={childPath}
            onSelectFile={onSelectFile}
          />
        </div>
      )}
    </li>
  );
}

function Leaf({ entry, onClick }: { entry: TreeEntry; onClick?: () => void }) {
  return (
    <li>
      <button
        type="button"
        disabled={!onClick}
        onClick={onClick}
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-slate-200 enabled:hover:bg-slate-800/60 disabled:cursor-default"
      >
      {entry.kind === "file" && <FileIcon />}
      {entry.kind === "symlink" && <LinkIcon />}
      {entry.kind === "submodule" && <GitIcon />}
      {entry.kind === "other" && <QuestionIcon />}
      <span className="truncate">{entry.name}</span>
      {"size" in entry ? (
        <span className="ml-auto shrink-0 text-xs text-slate-400">
          {formatBytes(entry.size)}
        </span>
      ) : null}
      </button>
    </li>
  );
}

function FilePreview({ owner, project, refValue, path, onBack }: {
  owner: string;
  project: string;
  refValue: string;
  path: string;
  onBack: () => void;
}) {
  const url = useMemo(() => {
    const base = import.meta.env.VITE_API_URL as string;
    const requestUrl = new URL(`${base}/project/${owner}/${project}/file`);
    requestUrl.searchParams.set("ref", refValue);
    requestUrl.searchParams.set("path", path);
    return requestUrl.toString();
  }, [owner, project, refValue, path]);
  const { data, error, isLoading } = useSWR<FileResponse>(["file", url], ([, requestUrl]) => apiFetcher(requestUrl));

  return (
    <div className="overflow-hidden rounded-lg border border-slate-700 bg-slate-900">
      <div className="flex min-w-0 items-center gap-3 border-b border-slate-700 px-3 py-2">
        <button type="button" onClick={onBack} className="shrink-0 rounded px-2 py-1 text-sm text-blue-400 hover:bg-slate-800">← Back</button>
        <span className="min-w-0 truncate font-mono text-sm">{path}</span>
        {data && <span className="ml-auto shrink-0 text-xs text-slate-400">{formatBytes(data.size)}</span>}
      </div>
      {isLoading ? (
        <RowSkeleton label={`Loading ${path}...`} />
      ) : error ? (
        <ErrorRow message={(error as Error).message.includes("413") ? "File is too large to preview (maximum 512 KiB)." : (error as Error).message.includes("415") ? "Binary files cannot be previewed." : `Failed to load file: ${(error as Error).message}`} />
      ) : (
        <pre className="max-h-[65vh] overflow-auto p-4 text-sm leading-6"><code>{data?.content}</code></pre>
      )}
    </div>
  );
}

/* ---------------------- Icons / UI bits ---------------------- */

function ChevronIcon({ open }: { open: boolean }) {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      className={`transition-transform ${open ? "rotate-90" : ""}`}
    >
      <path
        d="M9 6l6 6-6 6"
        stroke="#cbd5e1"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function FolderIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path
        d="M3 7a2 2 0 0 1 2-2h3l2 2h9a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z"
        stroke="#e2e8f0"
        strokeWidth="2"
      />
    </svg>
  );
}
function FileIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path
        d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12V8L14 2z"
        stroke="#e2e8f0"
        strokeWidth="2"
      />
      <path d="M14 2v6h6" stroke="#e2e8f0" strokeWidth="2" />
    </svg>
  );
}
function LinkIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path
        d="M10 13a5 5 0 0 0 7.07 0l2.83-2.83a5 5 0 0 0-7.07-7.07L10 5"
        stroke="#e2e8f0"
        strokeWidth="2"
      />
      <path
        d="M14 11a5 5 0 0 0-7.07 0L4.1 13.83a5 5 0 0 0 7.07 7.07L14 19"
        stroke="#e2e8f0"
        strokeWidth="2"
      />
    </svg>
  );
}
function GitIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path d="M3 12l9-9 9 9-9 9-9-9z" stroke="#e2e8f0" strokeWidth="2" />
      <circle cx="12" cy="7.5" r="1.5" fill="#e2e8f0" />
      <circle cx="12" cy="16.5" r="1.5" fill="#e2e8f0" />
      <circle cx="7.5" cy="12" r="1.5" fill="#e2e8f0" />
      <circle cx="16.5" cy="12" r="1.5" fill="#e2e8f0" />
    </svg>
  );
}
function QuestionIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path d="M9 9a3 3 0 1 1 3 3v2" stroke="#e2e8f0" strokeWidth="2" />
      <path d="M12 17h.01" stroke="#e2e8f0" strokeWidth="2" />
      <circle cx="12" cy="12" r="10" stroke="#e2e8f0" strokeWidth="2" />
    </svg>
  );
}

function RowSkeleton({ label }: { label: string }) {
  return (
    <div className="px-3 py-2 text-sm text-slate-400 animate-pulse">
      {label}
    </div>
  );
}

function ErrorRow({ message }: { message: string }) {
  return <div className="px-3 py-2 text-sm text-red-400">{message}</div>;
}

/* ---------------------- utils ---------------------- */

function joinPath(parent: string, name: string) {
  return parent ? `${parent.replace(/\/+$/, "")}/${name}` : name;
}

function formatBytes(bytes: number) {
  if (bytes === 0) return "0 B";
  const k = 1024,
    sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(i ? 1 : 0)} ${sizes[i]}`;
}
