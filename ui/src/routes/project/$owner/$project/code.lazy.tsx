import { createLazyFileRoute, useParams } from "@tanstack/react-router";
import useSWR from "swr";
import { useMemo, useState, Fragment } from "react";

export const Route = createLazyFileRoute("/project/$owner/$project/code")({
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

function CodeBrowser() {
  // @ts-ignore
  const { owner, project } = useParams({ strict: false });
  const [ref, setRef] = useState<string>("HEAD");

  return (
    <div className="w-full">
      {/* Header / Controls */}
      <div className="mb-4 flex items-center gap-3">
        <span className="text-sm text-slate-300">Ref</span>
        <input
          value={ref}
          onChange={(e) => setRef(e.target.value)}
          placeholder="HEAD or branch/tag/sha"
          className="rounded-md bg-slate-800 px-3 py-2 text-sm outline-none ring-1 ring-slate-700 focus:ring-slate-500"
        />
      </div>

      {/* Tree root */}
      <div className="rounded-lg border border-slate-700 bg-slate-900">
        <Tree owner={owner} project={project} refValue={ref} path="" />
      </div>
    </div>
  );
}

function Tree({
  owner,
  project,
  refValue,
  path,
}: {
  owner: string;
  project: string;
  refValue: string;
  path: string;
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
            />
          ) : (
            <Leaf entry={e} />
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
}: {
  owner: string;
  project: string;
  refValue: string;
  parentPath: string;
  name: string;
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
          />
        </div>
      )}
    </li>
  );
}

function Leaf({ entry }: { entry: TreeEntry }) {
  return (
    <li className="flex items-center gap-2 px-3 py-2 text-slate-200">
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
    </li>
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
