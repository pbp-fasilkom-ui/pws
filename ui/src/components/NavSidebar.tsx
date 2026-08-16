import { DoubleArrowRightIcon, ExitIcon, HomeIcon, PersonIcon, PlusIcon } from "@radix-ui/react-icons";
import { FC, ReactElement } from "react";
import { Button } from "./ui/button";
import { Link } from "@tanstack/react-router";
import { useAuth } from "@/contexts/AuthContext";
import useSWR from "swr";

export interface NavSidebarProps {
  className?: string
  onNavigate?: () => void
}

export default function NavSidebar({ className = "", onNavigate }: NavSidebarProps): ReactElement<FC<NavSidebarProps>> {
  const auth = useAuth()

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

  const { data: projects } = useSWR(`${import.meta.env.VITE_API_URL}/dashboard/project/`, apiFetcher)

  return (
    <aside className={`${className} h-full min-h-0 flex-col border-r border-slate-600 bg-[#020618]`}>
      <div className="flex h-16 items-center gap-3 px-5">
        <img className="h-9 w-9 shrink-0" src="/web/makara.png" alt="PWS" />
        <div className="min-w-0 leading-tight">
          <h1 className="truncate text-base font-semibold italic">PWS</h1>
          <p className="truncate text-xs text-slate-400">Pacil Web Service</p>
        </div>
      </div>
      <hr className="border-slate-600" />
      <div className="flex flex-col items-center justify-center px-6 py-4">
        <div className="flex items-center space-x-4 w-full">
          <PersonIcon className="h-6 w-6" />
          <div>
            <h1
              className="font-bold truncate"
            >
              {auth.user.name}
            </h1>
            <p className="text-slate-600">
              {auth.user.username}
            </p>
          </div>
        </div>
      </div>
      <hr className="border-slate-600" />
      <nav className="flex min-h-0 flex-1 flex-col items-center justify-start px-4 py-4 space-y-2 overflow-y-auto">
        <Link
          className="flex items-center space-x-4 w-full py-2 px-4 rounded-lg hover:bg-slate-700 transition-all"
          to="/"
          activeProps={{
            className: "bg-slate-700"
          }}
          onClick={onNavigate}
        >
          <HomeIcon className="w-4 h-4" />
          <span className="font-semibold text-sm">Home</span>
        </Link>
        {projects?.data?.map((item: any) => (
            <Link
              key={`${item.owner_name}-${item.name}`}
              className="flex items-center space-x-4 w-full py-2 px-4 rounded-lg hover:bg-slate-700 transition-all"
              href={`/project/${item.owner_name}/${item.name}`}
              to={`/project/$owner/$project`}
              params={{
                owner: item.owner_name,
                project: item.name
              }}
              activeProps={{
                className: "bg-slate-700"
              }}
              onClick={onNavigate}
            >
              <DoubleArrowRightIcon className="w-4 h-4" />
              <span className="min-w-0 truncate font-semibold text-sm">{item.owner_name}/{item.name}</span>
            </Link>
          ))}
      </nav>
      <hr className="border-slate-600" />
      <div className="flex flex-col items-center justify-center px-4 py-4 space-y-3">
        {projects?.data && projects.data.length > 0 && (
          <div className="w-full text-center text-xs text-slate-400">
            <span className="bg-slate-800 px-2 py-1 rounded">
              {projects?.owned_count || 0} / 3 owned projects
            </span>
          </div>
        )}
        <Link href="/create-project" to="/create-project" className="w-full" onClick={onNavigate}>
          <Button 
            variant="outline" 
            size="lg" 
            className={`w-full space-x-4 border-primary text-primary hover:bg-primary ${
              (projects?.owned_count || 0) >= 3 ? 'opacity-50 cursor-not-allowed' : ''
            }`}
            disabled={(projects?.owned_count || 0) >= 3}
          >
            <PlusIcon className="mr-2 h-4 w-4" /> 
            {(projects?.owned_count || 0) >= 3 ? 'Project Limit Reached' : 'Create New Project'}
          </Button>
        </Link>
        <Button
          variant="outline"
          size="lg"
          className="w-full border-red-400/60 text-red-300 hover:bg-red-500/10 hover:text-red-200"
          onClick={() => auth.handlers.logout()}
        >
          <ExitIcon className="mr-2 h-4 w-4" />
          Logout
        </Button>
      </div>
    </aside>
  )
}
