import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { createLazyFileRoute } from '@tanstack/react-router';
import { Controller, useForm } from 'react-hook-form';

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert';
import { ExclamationTriangleIcon } from '@radix-ui/react-icons';
import { useAuth } from '@/contexts/AuthContext';
import { useState } from 'react';
import { useSWRConfig } from 'swr';
import Spinner from '@/components/ui/spinner';
import useSWR from 'swr';

export const Route = createLazyFileRoute('/create-project')({
  component: NewProject,
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

function NewProject() {
  const auth = useAuth()
  const { mutate } = useSWRConfig()
  
  const { data: projects } = useSWR(`${import.meta.env.VITE_API_URL}/dashboard/project/`, apiFetcher)
  const projectCount = projects?.data?.length || 0
  const isAtLimit = projectCount >= 3

  const {
    formState: { isSubmitting },
    handleSubmit,
    register, 
    control 
  } = useForm()

  const [response, setResponse] = useState<{
    owner_name: string
    project_name: string
    domain: string
    git_username: string
    git_password: string
  }>()

  const [error, setError] = useState<{
    message: string
  }>()

  async function submitHandler({ owner, project }: any) {
    const response = await fetch(`${import.meta.env.VITE_API_URL}/project/new`, {
      credentials: "include",
      headers: {
        "Content-Type": "application/json"
      },
      method: "POST",
      body: JSON.stringify({
        owner,
        project,
      })
    })

    const data = await response.json()

    if (response.status >= 400) {
      setError(data)
      return
    }

    setResponse(data)
    setError(undefined)
    mutate(`${import.meta.env.VITE_API_URL}/dashboard/project/`)
  }

  return (
    <div className="relative min-h-full w-full">
      <div className="flex h-16 w-full items-center border-b border-slate-600 bg-[#020618]">
        <div className="px-4 md:px-8">
          <h1 className="text-2xl font-semibold">Create a New Project</h1>
        </div>
      </div>
      <div className="space-y-8 p-4 pb-16 md:p-8 md:pb-32">
        {isAtLimit && !response && (
          <Alert variant="default" className="border-red-400 text-red-400">
            <ExclamationTriangleIcon className="h-5 w-5 mt-0.5 !text-red-400" />
            <AlertTitle className="text-lg font-semibold">
              Project Limit Reached
            </AlertTitle>
            <AlertDescription>
              You have reached the maximum limit of 3 projects per user. Please delete an existing project to create a new one.
            </AlertDescription>
          </Alert>
        )}

        {projectCount > 0 && projectCount < 3 && !response && (
          <Alert variant="default" className="border-yellow-400 text-yellow-400">
            <ExclamationTriangleIcon className="h-5 w-5 mt-0.5 !text-yellow-400" />
            <AlertTitle className="text-lg font-semibold">
              Project Count: {projectCount} / 3
            </AlertTitle>
            <AlertDescription>
              You can create {3 - projectCount} more project{3 - projectCount !== 1 ? 's' : ''}.
            </AlertDescription>
          </Alert>
        )}

        {error && (
          <Alert variant="default" className="border-red-400 text-red-400">
            <ExclamationTriangleIcon className="h-5 w-5 mt-0.5 !text-red-400" />
            <AlertTitle className="text-lg font-semibold">
              Project Failed to Create
            </AlertTitle>
            <AlertDescription>
              {error.message}
            </AlertDescription>
          </Alert>
        )}

        <div className="space-y-4">
          <h1 className="font-semibold text-2xl">Project Information</h1>
          <form className="space-y-6" onSubmit={handleSubmit(submitHandler)}>
            <div className="flex max-w-2xl flex-col gap-4 sm:flex-row">
              <div className="space-y-2 sm:w-40 sm:shrink-0">
                <label className="text-slate-300">Owner</label>
                <Controller
                  name="owner"
                  control={control}
                  defaultValue={auth.user.username}
                  render={({ field }) => {
                    return <Select onValueChange={field.onChange} {...field} disabled={isAtLimit}>
                      <SelectTrigger className={`bg-slate-900 border-slate-600 bg-opacity-90 ${isAtLimit ? 'opacity-50' : ''}`}>
                        <SelectValue placeholder="Owner" />
                      </SelectTrigger>
                      <SelectContent className="border-slate-600">
                        <SelectItem value={auth.user.username}>{auth.user.username}</SelectItem>
                      </SelectContent>
                    </Select>;
                  }}
                />
              </div>
              <div className="min-w-0 flex-1 space-y-2">
                <label className="text-slate-300">Project Name</label>
                <Input 
                  className={`w-full bg-slate-900 border-slate-600 bg-opacity-90 ${isAtLimit ? 'opacity-50' : ''}`}
                  {...register("project")} 
                  disabled={isAtLimit}
                />
              </div>
            </div>
            {!isSubmitting && !isAtLimit ? (
              <Button size="lg" className="w-full text-white sm:w-64">
                Create New Project
              </Button>
            ) : isAtLimit ? (
              <Button disabled size="lg" className="w-full text-white opacity-50 sm:w-64">
                Project Limit Reached
              </Button>
            ) : (
              <Button disabled size="lg" className="w-full text-white sm:w-64">
                <Spinner className="mr-2" /> Creating Project
              </Button>
            )}
          </form>
        </div>

        {response && (
          <div className="space-y-8">
            <h1 className="font-semibold text-2xl">Project Configuration</h1>
            <div className="space-y-4">
              <div className="space-y-2">
                <h2 className="font-semibold text-xl">Project Credentials</h2>
                <p>
                  This credential will be used to identify your ownership authenticity when deploying your code. When executing the command, you will be asked for this credential. <span className="text-red-400">DO NOT SHARE</span> the credential, as this will allow other people to push code to this project.
                </p>
              </div>
              <Alert variant="default" className="border-red-600 text-red-200 bg-red-600">
                <ExclamationTriangleIcon className="h-5 w-5 mt-0.5" />
                <AlertTitle className="text-lg font-semibold">
                  PLEASE COPY THE CREDENTIAL BELOW
                </AlertTitle>
                <AlertDescription>
                  You will need this credential to deploy your code as you will be asked later. Please copy the credential and save it AS YOU WILL NOT BE ABLE TO ACCESS IT LATER.
                </AlertDescription>
              </Alert>
              <div className="w-full overflow-x-auto rounded-lg bg-slate-900 p-4 md:p-8">
                <pre className="min-w-max">
                  Username: {response.git_username}
                </pre>
                <pre className="min-w-max">
                  Password: {response.git_password}
                </pre>
              </div>
            </div>
            <div className="space-y-4">
              <div className="space-y-2">
                <h2 className="font-semibold text-xl">Project Command</h2>
                <p>
                  You will need to use this command to deploy your code. If you have done this, in the future you will need to just use the third line only.
                </p>
              </div>
              <div className="w-full overflow-x-auto rounded-lg bg-slate-900 p-4 md:p-8">
                <pre className="min-w-max">
                  git remote add pws {response.domain}
                </pre>
                <pre className="min-w-max">
                  git branch -M master
                </pre>
                <pre className="min-w-max">
                  git push pws master
                </pre>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
