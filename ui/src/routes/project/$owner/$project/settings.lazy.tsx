import { Button } from '@/components/ui/button'
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from '@/components/ui/dialog'
import { createLazyFileRoute, useNavigate, useParams } from '@tanstack/react-router'
import toast from 'react-hot-toast'
import { useEffect, useState } from 'react'

export const Route = createLazyFileRoute('/project/$owner/$project/settings')({
  component: ProjectDashboardSettings
})

interface GitCredentials {
  git_username: string
  git_url: string
  project_name: string
  owner_name: string
}

interface RegeneratePasswordResponse {
  git_username: string
  git_password: string
  git_url: string
  message: string
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
  )
}

function ProjectDashboardSettings() {
  // @ts-ignore
  const { owner, project } = useParams({ strict: false })
  const navigate = useNavigate()
  const [gitCredentials, setGitCredentials] = useState<GitCredentials | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [regeneratedPassword, setRegeneratedPassword] = useState<string | null>(null)
  const [isRegenerating, setIsRegenerating] = useState(false)

  useEffect(() => {
    fetchGitCredentials()
  }, [owner, project])

  async function fetchGitCredentials() {
    try {
      setIsLoading(true)
      const response = await apiFetcher(`${import.meta.env.VITE_API_URL}/project/${owner}/${project}/git-credentials`)
      if (response.ok) {
        const data: GitCredentials = await response.json()
        setGitCredentials(data)
      } else {
        console.error('Failed to fetch git credentials')
        toast.error('Failed to fetch git credentials')
      }
    } catch (error) {
      console.error('Error fetching git credentials:', error)
      toast.error('Error fetching git credentials')
    } finally {
      setIsLoading(false)
    }
  }

  function copyToClipboard(text: string, type: string) {
    navigator.clipboard.writeText(text).then(() => {
      toast.success(`${type} copied to clipboard!`, {
        position: "bottom-right",
        style: {
          backgroundColor: "#020817",
          color: "white"
        }
      })
    }).catch(() => {
      toast.error(`Failed to copy ${type}`)
    })
  }

  async function handleRegeneratePassword() {
    try {
      setIsRegenerating(true)
      const response = await apiFetcher(`${import.meta.env.VITE_API_URL}/project/${owner}/${project}/regenerate-git-password`, {
        method: "POST",
      })
      
      if (response.ok) {
        const data: RegeneratePasswordResponse = await response.json()
        setRegeneratedPassword(data.git_password)
        toast.success(data.message, {
          position: "bottom-right",
          style: {
            backgroundColor: "#020817",
            color: "white"
          }
        })
      } else {
        const errorData = await response.json()
        toast.error(errorData.message || 'Failed to regenerate password')
      }
    } catch (error) {
      console.error('Error regenerating password:', error)
      toast.error('Error regenerating password')
    } finally {
      setIsRegenerating(false)
    }
  }

  async function handleProjectDelete() {
    const deleteRequest = apiFetcher(`${import.meta.env.VITE_API_URL}/project/${owner}/${project}/delete`, {
      method: "POST",
    }).then(async (res) => {
      if (res.ok) {
        return res
      } else {
        const response = await res.json()

        return response.message
      }
    })

    toast.promise(deleteRequest, {
      loading: "Deleting project...",
      success: (_) => {
        navigate({ from: location.pathname, to: "/" })
        return "Successfully deleted project"
      },
      error: (_) => {
        navigate({ from: location.pathname, to: "/" })
        return "An error might have occurred during project deletion, please check your project dashboard"
      }
    }, {
      position: "bottom-right",
      style: {
        backgroundColor: "#020817",
        color: "white"
      }
    })
  }

  return (
    <div className="space-y-4 w-full">
      <div className="text-sm space-y-1">
        <h1 className="text-xl font-semibold">Project Settings</h1>
        <p className="text-sm">List of all the possible settings you can do in this project</p>
      </div>
      <div className="w-full space-y-6">
        {/* Git Credentials Section */}
        <div>
          <h1 className="font-medium">Git Credentials</h1>
          <p className="text-sm">Your git credentials for pushing to this project</p>
          
          {isLoading ? (
            <div className="mt-4 p-4 bg-slate-800 rounded-lg">
              <p className="text-sm text-slate-400">Loading git credentials...</p>
            </div>
          ) : gitCredentials ? (
            <div className="mt-4 space-y-3">
              {/* Git URL */}
              <div className="p-4 bg-slate-800 rounded-lg space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium text-slate-300">Git URL</span>
                  <Button 
                    size="sm" 
                    variant="outline" 
                    onClick={() => copyToClipboard(gitCredentials.git_url, 'Git URL')}
                    className="h-6 px-2 text-xs"
                  >
                    Copy
                  </Button>
                </div>
                <code className="block text-sm font-mono text-white bg-slate-900 p-2 rounded">
                  {gitCredentials.git_url}
                </code>
              </div>

              {/* Username */}
              <div className="p-4 bg-slate-800 rounded-lg space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium text-slate-300">Username</span>
                  <Button 
                    size="sm" 
                    variant="outline" 
                    onClick={() => copyToClipboard(gitCredentials.git_username, 'Username')}
                    className="h-6 px-2 text-xs"
                  >
                    Copy
                  </Button>
                </div>
                <code className="block text-sm font-mono text-white bg-slate-900 p-2 rounded">
                  {gitCredentials.git_username}
                </code>
              </div>

              {/* Password Section */}
              <div className="p-4 bg-slate-800 rounded-lg space-y-3">
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium text-slate-300">Git Password</span>
                  <Button 
                    size="sm" 
                    variant="outline" 
                    onClick={handleRegeneratePassword}
                    disabled={isRegenerating}
                    className="h-6 px-2 text-xs"
                  >
                    {isRegenerating ? "Regenerating..." : "Regenerate"}
                  </Button>
                </div>
                
                {regeneratedPassword ? (
                  <div className="space-y-2">
                    <div className="flex items-center justify-between">
                      <span className="text-xs text-green-400">New Password Generated:</span>
                      <Button 
                        size="sm" 
                        variant="outline" 
                        onClick={() => copyToClipboard(regeneratedPassword, 'New Password')}
                        className="h-6 px-2 text-xs"
                      >
                        Copy
                      </Button>
                    </div>
                    <code className="block text-sm font-mono text-green-400 bg-slate-900 p-2 rounded border border-green-500/20">
                      {regeneratedPassword}
                    </code>
                    <div className="flex items-start space-x-2 mt-2">
                      <svg className="w-4 h-4 text-red-500 mt-0.5" fill="currentColor" viewBox="0 0 20 20">
                        <path fillRule="evenodd" d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z" clipRule="evenodd" />
                      </svg>
                      <p className="text-xs text-red-400">
                        <strong>IMPORTANT:</strong> Save this password now! It won't be shown again and your old password is now invalid.
                      </p>
                    </div>

                    {/* Remote reset helper */}
                    <div className="p-4 bg-slate-900 rounded-lg border border-slate-800 space-y-2 mt-3">
                      <div className="flex items-center justify-between">
                        <span className="text-sm font-medium text-slate-300">Reset Git Remote (refresh credentials)</span>
                      </div>

                      {/* Remove remote */}
                      <div className="flex items-center justify-between mt-1">
                        <span className="text-xs text-slate-400">Remove existing remote named <code className='font-mono'>pws</code>:</span>
                        <Button 
                          size="sm" 
                          variant="outline" 
                          onClick={() => copyToClipboard('git remote remove pws', 'Remove remote command')}
                          className="h-6 px-2 text-xs"
                        >
                          Copy
                        </Button>
                      </div>
                      <code className="block text-xs font-mono text-white bg-black/40 p-2 rounded">git remote remove pws</code>

                      {/* Add remote */}
                      <div className="flex items-center justify-between mt-2">
                        <span className="text-xs text-slate-400">Add remote again with updated URL:</span>
                        <Button 
                          size="sm" 
                          variant="outline" 
                          onClick={() => copyToClipboard(`git remote add pws ${gitCredentials.git_url}`, 'Add remote command')}
                          className="h-6 px-2 text-xs"
                        >
                          Copy
                        </Button>
                      </div>
                      <code className="block text-xs font-mono text-white bg-black/40 p-2 rounded">{`git remote add pws ${gitCredentials.git_url}`}</code>
                    </div>
                  </div>
                ) : (
                  <div className="flex items-start space-x-2">
                    <svg className="w-4 h-4 text-yellow-500 mt-0.5" fill="currentColor" viewBox="0 0 20 20">
                      <path fillRule="evenodd" d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z" clipRule="evenodd" />
                    </svg>
                    <div>
                      <p className="text-sm font-medium text-yellow-500">Password Security</p>
                      <p className="text-xs text-slate-400 mt-1">
                        Your git password was shown only once during project creation for security reasons. 
                        Click "Regenerate" to create a new password.
                      </p>
                    </div>
                  </div>
                )}
              </div>
            </div>
          ) : (
            <div className="mt-4 p-4 bg-red-800/20 border border-red-500/20 rounded-lg">
              <p className="text-sm text-red-400">Failed to load git credentials</p>
            </div>
          )}
        </div>

        <div>
          <h1 className="font-medium">Project Controls</h1>
          <p className="text-sm">Actions that you can take in this project</p>
        </div>
        <div className="flex space-x-4">
          <Dialog>
            <DialogTrigger>
              <Button className="bg-red-600 text-foreground hover:bg-red-700">
                <svg width="20" height="20" className="mr-1" viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg">
                  <path d="M6.81462 9.6643L6.57837 9.90056L6.81518 10.1363L8.35337 11.6672L6.82296 13.1976L6.58725 13.4333L6.82296 13.669L7.99796 14.844L8.23366 15.0797L8.46936 14.844L10.0003 13.3131L11.5313 14.844L11.767 15.0797L12.0027 14.844L13.1777 13.669L13.4134 13.4333L13.1777 13.1976L11.6467 11.6667L13.1777 10.1357L13.4134 9.9L13.1777 9.6643L12.0027 8.4893L11.767 8.2536L11.5313 8.4893L9.99977 10.0208L8.46047 8.48874L8.22477 8.25415L7.98962 8.4893L6.81462 9.6643ZM12.6813 3.56904L12.7789 3.66667H12.917H15.5003V4.66667H4.50033V3.66667H7.08366H7.22173L7.31936 3.56904L8.05506 2.83333H11.9456L12.6813 3.56904ZM6.66699 17.1667C5.93442 17.1667 5.33366 16.5659 5.33366 15.8333V6.16667H14.667V15.8333C14.667 16.5659 14.0662 17.1667 13.3337 17.1667H6.66699Z" fill="white" stroke="white" stroke-width="0.666667" />
                </svg>
                Delete Project
              </Button>
            </DialogTrigger>
            <DialogContent className="text-white">
              <DialogHeader>
                <DialogTitle>Delete Project - Are you absolutely sure?</DialogTitle>
                <DialogDescription>
                  This action cannot be undone. This will permanently delete your project and associated database.
                  You will have to push your project again to redeploy.
                </DialogDescription>
                <DialogFooter>
                  <DialogClose>
                    <Button size="lg" className="text-foreground">
                      No, Don't
                    </Button>
                  </DialogClose>
                  <Button onClick={handleProjectDelete} size="lg" className="bg-red-600 text-foreground hover:bg-red-700">
                    Yes, Delete My Project
                  </Button>
                </DialogFooter>
              </DialogHeader>
            </DialogContent>
          </Dialog>
        </div>
      </div>
    </div>
  )
}