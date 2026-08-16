import { Button } from '@/components/ui/button';
import { Link, createLazyFileRoute } from '@tanstack/react-router';
import useSWR from 'swr';


export const Route = createLazyFileRoute('/')({
  component: Index,
})

function NoProject() {
  return (
    <div className="flex min-h-[calc(100vh-10rem)] flex-col items-center justify-center py-8">
      <img className="w-56 max-w-full sm:w-72" src="/web/no-project.svg" alt="No projects" />
      <div className="flex flex-col justify-center items-center space-y-4">
        <div className="space-y-2 text-center">
          <h1 className="text-2xl font-semibold md:text-3xl">You currently have no projects</h1>
          <h2 className="text-lg">Let's create one easily</h2>
        </div>
        <Link href="/create-project" to="/create-project">
          <Button size="lg" className="text-white">
            Create New Project
          </Button>
        </Link>
      </div>
    </div>
  )
}

function ProjectListSkeleton() {
  return (
    <div className="space-y-6" aria-label="Loading projects" aria-busy="true">
      <div className="h-8 w-36 animate-pulse rounded bg-slate-800" />
      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2 xl:gap-8">
        <div className="h-32 animate-pulse rounded-lg border border-slate-700 bg-slate-900/70" />
        <div className="hidden h-32 animate-pulse rounded-lg border border-slate-700 bg-slate-900/70 xl:block" />
      </div>
    </div>
  )
}

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

function Index() {
  const { data: projects, isLoading } = useSWR(`${import.meta.env.VITE_API_URL}/dashboard/project/`, apiFetcher)

  return (
    <div className="relative min-h-full w-full">
      <div className="flex h-16 w-full items-center border-b border-slate-600 bg-[#020618]">
        <div className="px-4 md:px-8">
          <h1 className="text-2xl font-semibold">Home</h1>
        </div>
      </div>

      <div className="space-y-8 p-4 pb-16 md:p-8 md:pb-32">
        {isLoading || !projects ? <ProjectListSkeleton /> : !projects.data?.length ? <NoProject /> : (
          <>
            <h1 className="font-semibold text-2xl">Project List</h1>
            <div className="grid grid-cols-1 gap-4 xl:grid-cols-2 xl:gap-8">
              {projects?.data?.map((item: any) => (
                <Link
                  href={`/project/${item.owner_name}/${item.name}/`}
                  to="/project/$owner/$project"
                  params={{
                    owner: item.owner_name,
                    project: item.name
                  }}
                  className="bg-slate-900 border p-8 rounded-lg space-y-4 border-slate-500 hover:border-blue-400 transition-all cursor-pointer"
                >
                  <div className="space-y-1">
                    <h1 className="text-lg font-semibold">{item.owner_name}/{item.name}</h1>
                    <h2 className="text-sm text-blue-400">{item.id}</h2>
                  </div>
                </Link>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  )
}
