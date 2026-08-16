import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { ChevronDownIcon, ExclamationTriangleIcon } from '@radix-ui/react-icons'
import { createFileRoute, Link, redirect, useParams } from '@tanstack/react-router'

async function checkProjectAccess(owner: string, project: string) {
  try {
    const response = await fetch(
      `${(import.meta as any).env.VITE_API_URL}/project/${owner}/${project}/access`,
      { credentials: 'include' },
    )
    return response.ok
  } catch {
    return false
  }
}

export const Route = createFileRoute('/project/$owner/$project/troubleshooting')({
  beforeLoad: async ({ params }) => {
    if (!(await checkProjectAccess(params.owner, params.project))) {
      throw redirect({ to: '/', search: { error: 'access_denied' } })
    }
  },
  component: Troubleshooting,
})

const sqliteReset = `rm -f db.sqlite3
python manage.py migrate --noinput`

const postgresReset = `python manage.py shell -c "from django.db import connection; schema='NAMA_SCHEMA_ANDA'; q=connection.ops.quote_name(schema); c=connection.cursor(); c.execute(f'DROP SCHEMA {q} CASCADE'); c.execute(f'CREATE SCHEMA {q}')"
python manage.py migrate --noinput`

function CommandBlock({ children }: { children: string }) {
  return (
    <pre className="overflow-x-auto rounded-lg border border-slate-700 bg-slate-950 p-4 text-sm leading-6 text-slate-200">
      <code>{children}</code>
    </pre>
  )
}

function Troubleshooting() {
  // @ts-ignore
  const { owner, project } = useParams({ strict: false })

  return (
    <div className="max-w-4xl space-y-6 pb-16">
      <div className="space-y-1">
        <h1 className="text-xl font-semibold">Troubleshooting</h1>
        <p className="text-sm text-slate-400">Common recovery guides for your project.</p>
      </div>

      <details className="group rounded-lg border border-slate-700 bg-slate-900/80">
        <summary className="flex cursor-pointer list-none items-center justify-between p-4 font-semibold md:px-6">
          Migration errors
          <ChevronDownIcon className="transition-transform group-open:rotate-180" />
        </summary>
        <div className="space-y-5 border-t border-slate-700 p-4 md:p-6">
          <Alert className="border-yellow-500/70 bg-yellow-950/30 text-yellow-100">
            <ExclamationTriangleIcon className="mt-0.5 h-5 w-5 text-yellow-400" />
            <AlertTitle className="font-semibold">Reset permanently deletes project data</AlertTitle>
            <AlertDescription className="text-yellow-100/80">
              Retry <code>python manage.py migrate --noinput</code> first.
            </AlertDescription>
          </Alert>

          <p className="text-sm text-slate-300">
            Run the following commands in the PWS Terminal for your database.
          </p>

          <section className="space-y-2">
            <h2 className="font-semibold">SQLite</h2>
            <CommandBlock>{sqliteReset}</CommandBlock>
          </section>

          <section className="space-y-2">
            <h2 className="font-semibold">External PostgreSQL</h2>
            <p className="text-sm text-slate-400">
              Replace <code>NAMA_SCHEMA_ANDA</code> with your assigned schema.
            </p>
            <CommandBlock>{postgresReset}</CommandBlock>
          </section>

          <Link to="/project/$owner/$project/terminal" params={{ owner, project }}>
            <Button className="text-white">Go to PWS Terminal</Button>
          </Link>
        </div>
      </details>
    </div>
  )
}
