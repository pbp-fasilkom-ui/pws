import { createLazyFileRoute, useRouter } from '@tanstack/react-router';
import {
    Card,
    CardContent,
    CardFooter,
    CardHeader,
    CardTitle,
} from "@/components/ui/card"
import { Label } from '@/components/ui/label';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert';
import { CheckCircledIcon, ExclamationTriangleIcon } from '@radix-ui/react-icons';
import { useForm } from 'react-hook-form';
import { useState } from 'react';
import Spinner from '@/components/ui/spinner';

export const Route = createLazyFileRoute('/register')({
    component: Register,
})

function Register() {
    const { register, handleSubmit, formState: { isSubmitting } } = useForm()
    const router = useRouter()

    const [error, setError] = useState({ message: "", error_type: "" })

    async function submitHandler(data: any) {
        const request = await fetch(`${import.meta.env.VITE_API_URL}/register`, {
            method: "POST",
            headers: {
                "Content-Type": "application/json"
            },
            body: JSON.stringify({
                username: data.username,
                password: data.password,
                name: data.name,
            })
        })

        if (request.status >= 400) {
            const data = await request.json()
            setError(data)
        } else {
            router.navigate({ from: "/register", to: "/login" })
        }
    }

    return (
        <form onSubmit={handleSubmit(submitHandler)} className="flex flex-col w-full h-full min-h-screen justify-center items-center space-y-8">
            {error.message !== "" && (
                <Alert variant="default" className="max-w-lg w-full border-red-400 text-red-400">
                    <ExclamationTriangleIcon className="h-5 w-5 mt-0.5 !text-red-400" />
                    <AlertTitle className="text-lg font-semibold">
                        Registration Failed
                    </AlertTitle>
                    <AlertDescription>
                        {error.error_type === "ValidationError" ? (
                            <ul>
                                {error.message.split("\n").map((item) => (
                                    <li key={item}>{item}</li>
                                ))}
                            </ul>
                        ) : error.message}
                    </AlertDescription>
                </Alert>
            )}

            <Card className="max-w-lg w-full bg-slate-900 border-slate-600 p-2">
                <CardHeader>
                    <CardTitle className="text-center text-3xl">Register Account</CardTitle>
                </CardHeader>
                <CardContent className="gap-4 flex flex-col items-center justify-center space-y-2">
                    <div className="grid w-full max-w-sm items-center gap-1.5">
                        <Label className="text-md" htmlFor="username">Username</Label>
                        <Input type="text" id="username" placeholder="Username" {...register("username", { required: true })} />
                    </div>
                    <div className="grid w-full max-w-sm items-center gap-1.5">
                        <Label className="text-md" htmlFor="password">Password</Label>
                        <Input type="password" id="password" placeholder="Password" {...register("password", { required: true })} />
                    </div>
                    <div className="grid w-full max-w-sm items-center gap-1.5">
                        <Label className="text-md" htmlFor="fullName">Full Name</Label>
                        <Input type="text" id="fullName" placeholder="Full Name" {...register("name", { required: true })} />
                    </div>

                </CardContent>
                <CardFooter className="flex flex-col items-center justify-center space-y-4">
                    {!isSubmitting ? (
                        <Button size="lg" className="text-foreground w-2/3">
                            Register
                        </Button>
                    ) : (
                        <Button disabled size="lg" className="text-foreground w-2/3">
                            <Spinner className="mr-2" /> Registering Your Account
                        </Button>
                    )}
                </CardFooter>
            </Card>
        </form>
    )
}