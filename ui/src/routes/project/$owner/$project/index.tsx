import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { ReloadIcon } from '@radix-ui/react-icons'
import { Link, createFileRoute, useNavigate, useParams, redirect } from '@tanstack/react-router'
import useSWR, { useSWRConfig } from 'swr'
import toast from 'react-hot-toast'
import { useState } from 'react'

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

export const Route = createFileRoute('/project/$owner/$project/')({
  beforeLoad: async ({ params }: { params: { owner: string; project: string } }) => {
    const hasAccess = await checkProjectAccess(params.owner, params.project);
    if (!hasAccess) {
      throw redirect({ to: '/', search: { error: 'access_denied' } });
    }
  },
  component: ProjectDashboardIndex
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

function BuildBadge({ text }: { text: string }) {
  function getVariant() {
    if (text === "SUCCESSFUL") return "bg-green-700"
    if (text === "FAILED") return "bg-red-700"
    if (text === "BUILDING") return "bg-yellow-700"
    return "bg-slate-700"
  }

  return (
    <Badge className={`${getVariant()} text-white rounded-full font-medium`}>
      {text.charAt(0).toUpperCase() + text.toLowerCase().slice(1)}
    </Badge>
  )
}

function ProjectDashboardIndex() {
  // @ts-ignore
  const { owner, project } = useParams({ strict: false })
  const navigate = useNavigate()
  const { mutate } = useSWRConfig()
  const domain = import.meta.env.VITE_API_URL.match(/((.*):\/\/(.*)\/)/)?.[0]
  const buildsUrl = `${import.meta.env.VITE_API_URL}/project/${owner}/${project}/builds/`
  const [redeployOpen, setRedeployOpen] = useState(false)
  const [isRedeploying, setIsRedeploying] = useState(false)

  const { data: builds, isLoading } = useSWR(buildsUrl, apiFetcher)
  const hasActiveBuild = builds?.data?.some(
    (build: any) => build.status === 'PENDING' || build.status === 'BUILDING',
  )
  const redeployableBuild = builds?.data?.find(
    (build: any) => build.branch && build.commit_sha,
  )

  async function handleRedeploy() {
    setIsRedeploying(true)

    try {
      const response = await fetch(
        `${import.meta.env.VITE_API_URL}/project/${owner}/${project}/redeploy`,
        {
          method: 'POST',
          credentials: 'include',
          headers: { 'Content-Type': 'application/json' },
        },
      )
      const data = await response.json().catch(() => ({}))

      if (!response.ok) {
        throw new Error(data.message || 'Failed to queue redeploy')
      }

      setRedeployOpen(false)
      await mutate(buildsUrl)
      toast.success('Redeploy queued successfully', {
        position: 'bottom-right',
      })
      navigate({
        to: '/project/$owner/$project/build/$buildId',
        params: { owner, project, buildId: data.build_id },
      })
    } catch (error) {
      toast.error(error instanceof Error ? error.message : 'Failed to queue redeploy', {
        position: 'bottom-right',
      })
    } finally {
      setIsRedeploying(false)
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-col justify-between gap-4 text-sm sm:flex-row sm:items-start">
        <div className="space-y-1">
          <h1 className="text-xl font-medium">Project Builds</h1>
          <p>List of all build logs of this project</p>
        </div>

        <Dialog open={redeployOpen} onOpenChange={setRedeployOpen}>
          <DialogTrigger asChild>
            <Button
              size="lg"
              variant="outline"
              className="w-full text-foreground sm:w-auto"
              disabled={!redeployableBuild || hasActiveBuild || isRedeploying}
            >
              <ReloadIcon className="mr-2" />
              {hasActiveBuild ? 'Build in progress' : 'Redeploy'}
            </Button>
          </DialogTrigger>
          <DialogContent className="text-white">
            <DialogHeader>
              <DialogTitle>Redeploy latest revision?</DialogTitle>
              <DialogDescription>
                This will rebuild the latest recorded revision on the PWS build queue.
                {redeployableBuild && (
                  <span className="mt-2 block font-mono text-xs">
                    {redeployableBuild.branch} · {redeployableBuild.commit_sha.slice(0, 7)}
                  </span>
                )}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <DialogClose asChild>
                <Button variant="outline">Cancel</Button>
              </DialogClose>
              <Button onClick={handleRedeploy} disabled={isRedeploying}>
                {isRedeploying ? 'Queueing...' : 'Redeploy'}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>

      {isLoading ? (
        <div className="flex justify-center items-center py-16">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500"></div>
        </div>
      ) : (
        builds?.data?.length > 0 ? (
          <div className="w-full flex flex-col gap-4">
            {builds.data.map((build: { id: string, status: string, created_at: string, branch?: string, commit_sha?: string }) => (
              <Link
                to="/project/$owner/$project/build/$buildId"
                params={{ owner, project, buildId: build.id }}
              >
                <div className="bg-slate-900 border p-8 rounded-lg space-y-4 border-slate-500 hover:border-blue-400 transition-all cursor-pointer">
                  <div className="space-y-1">
                    <h1 className="text-lg font-semibold">{build.id}</h1>
                    <div className="flex flex-wrap items-center gap-2 text-sm text-slate-400">
                      {build.branch && <span>{build.branch}</span>}
                      {build.commit_sha && <code>{build.commit_sha.slice(0, 7)}</code>}
                      <span>Started at {build.created_at}</span>
                    </div>
                  </div>

                  <BuildBadge text={build.status} />
                </div>
              </Link>
            ))}
          </div>
        ) : (
          <>
            <p className="text-sm text-blue-400">
              You have not published a build to your project. Please use the following command in your project’s folder to push an existing app to this project.
            </p>
            <div className="w-full p-8 bg-slate-900 rounded-lg">
              <pre>
                git remote add pws {domain}{owner}/{project}
              </pre>
              <pre>
                git branch -M master
              </pre>
              <pre>
                git push pws master
              </pre>
            </div>
          </>
        )
      )}
    </div>
  )
}
