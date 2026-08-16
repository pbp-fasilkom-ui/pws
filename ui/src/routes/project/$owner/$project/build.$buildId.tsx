import { createFileRoute, useParams, redirect } from '@tanstack/react-router'
import useSWR from 'swr'
import { useSWRConfig } from 'swr'
import { useEffect, useRef, useState } from 'react'

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

export const Route = createFileRoute('/project/$owner/$project/build/$buildId')({
  beforeLoad: async ({ params }: { params: { owner: string; project: string; buildId: string } }) => {
    const hasAccess = await checkProjectAccess(params.owner, params.project);
    if (!hasAccess) {
      throw redirect({ to: '/', search: { error: 'access_denied' } });
    }
  },
  component: ProjectViewBuildLog
})

const apiFetcher = (input: URL | RequestInfo, options?: RequestInit) => {
  return fetch(
    input,
    {
      ...options,
      redirect: "follow",
      credentials: "include",
      headers: {
        "Content-Type": "application/json"
      },
    }
  ).then(res => res.json())
}

function ProjectViewBuildLog() {
  // @ts-ignore
  const { owner, project, buildId } = useParams({ strict: false })
  const { mutate } = useSWRConfig()

  const buildUrl = `${import.meta.env.VITE_API_URL}/project/${owner}/${project}/builds/${buildId}`
  const buildsUrl = `${import.meta.env.VITE_API_URL}/project/${owner}/${project}/builds/`
  const { data: build, isLoading } = useSWR(buildUrl, apiFetcher)
  const [logs, setLogs] = useState('')
  const [status, setStatus] = useState<string | undefined>()
  const [queuePosition, setQueuePosition] = useState<number | null>(null)
  const logContainer = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (build?.logs !== undefined) setLogs(build.logs)
    if (build?.status !== undefined) setStatus(build.status)
  }, [build?.logs, build?.status])

  useEffect(() => {
    const url = `${import.meta.env.VITE_API_URL}/project/${owner}/${project}/builds/${buildId}/events`
    const events = new EventSource(url, { withCredentials: true })

    events.addEventListener('snapshot', (message) => {
      const payload = JSON.parse(message.data)
      setLogs(payload.logs)
    })
    events.addEventListener('log', (message) => {
      const payload = JSON.parse(message.data)
      setLogs((current) => current + payload.chunk)
    })
    events.addEventListener('status', (message) => {
      const payload = JSON.parse(message.data)
      setStatus(payload.status)
      if (payload.status === 'successful' || payload.status === 'failed') {
        // Refresh both the finalized build data and the shared project build list.
        // The latter also updates the project status and "View Project" action
        // rendered by the parent route without requiring a page reload.
        void Promise.all([mutate(buildUrl), mutate(buildsUrl)])
        events.close()
      }
    })
    events.addEventListener('queue', (message) => {
      const payload = JSON.parse(message.data)
      setQueuePosition(payload.position)
    })

    return () => events.close()
  }, [owner, project, buildId, buildUrl, buildsUrl, mutate])

  useEffect(() => {
    if (logContainer.current) {
      logContainer.current.scrollTop = logContainer.current.scrollHeight
    }
  }, [logs])

  return (
    <div className="space-y-4">
      <div className="text-sm space-y-1">
        <h1 className="text-xl font-medium">Build Logs</h1>
        <p>Build ID: {build?.id}</p>
        <p className="text-slate-400">
          Status: {status ?? (isLoading ? 'loading' : 'unknown')}
          {status === 'pending' && queuePosition !== null ? ` · Queue #${queuePosition}` : ''}
        </p>
        {(build?.branch || build?.commit_sha) && (
          <p className="text-slate-400">
            {build?.branch ?? "Unknown branch"}
            {build?.commit_sha ? ` · ${build.commit_sha.slice(0, 7)}` : ""}
          </p>
        )}
      </div>
      <div ref={logContainer} className="w-full p-8 bg-slate-900 rounded-lg max-h-96 overflow-y-auto overflow-x-hidden">
        <pre className="w-full space-x-4 whitespace-pre-wrap">
          {logs}
        </pre>
      </div>
    </div>
  )
}
