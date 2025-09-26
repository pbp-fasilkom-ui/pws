import { Badge } from '@/components/ui/badge'
import { Link, createLazyFileRoute, useParams, redirect } from '@tanstack/react-router'
import useSWR from 'swr'

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

export const Route = createLazyFileRoute('/project/$owner/$project/')({
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
  const domain = import.meta.env.VITE_API_URL.match(/((.*):\/\/(.*)\/)/)?.[0]

  const { data: builds, isLoading } = useSWR(`${import.meta.env.VITE_API_URL}/project/${owner}/${project}/builds/`, apiFetcher)

  return (
    <div className="space-y-4">
      <div className="text-sm space-y-1">
        <h1 className="text-xl font-medium">Project Builds</h1>
        <p>List of all build logs of this project</p>
      </div>

      {isLoading ? (
        <div className="flex justify-center items-center py-16">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500"></div>
        </div>
      ) : (
        builds?.data?.length > 0 ? (
          <div className="w-full flex flex-col gap-4">
            {builds.data.map((build: { id: string, status: string, created_at: string }) => (
              <Link
                to="/project/$owner/$project/build/$buildId"
                params={{ owner, project, buildId: build.id }}
              >
                <div className="bg-slate-900 border p-8 rounded-lg space-y-4 border-slate-500 hover:border-blue-400 transition-all cursor-pointer">
                  <div className="space-y-1">
                    <h1 className="text-lg font-semibold">{build.id}</h1>
                    <h2 className="text-sm text-slate-400">Started at {build.created_at}</h2>
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