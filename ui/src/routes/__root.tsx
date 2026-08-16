import AuthNavbar from "@/components/AuthNavbar";
import NavSidebar from "@/components/NavSidebar";
import AuthProvider from "@/contexts/AuthContext";
import {
  createRootRoute,
  Outlet,
  useRouterState,
} from "@tanstack/react-router";
import { Toaster } from "react-hot-toast";
// import { TanStackRouterDevtools } from '@tanstack/router-devtools'
import { useEffect } from "react";
import { Cross1Icon, HamburgerMenuIcon } from "@radix-ui/react-icons";
import { useState } from "react";

export const Route = createRootRoute({
  component: RootLayout,
});

function RootLayout() {
  const routerState = useRouterState();
  const [mobileNavOpen, setMobileNavOpen] = useState(false);

  const isAuthRoute =
    routerState.location.pathname === "/login" ||
    routerState.location.pathname === "/register" ||
    routerState.location.pathname === "/sso";

  useEffect(() => setMobileNavOpen(false), [routerState.location.pathname]);

  return (
    <AuthProvider>
      <Toaster />
      <div className="min-h-screen w-full circle-bg text-foreground">
        {isAuthRoute ? (
          <>
            <AuthNavbar />
            <Outlet />
          </>
        ) : (
          <div className="flex min-h-screen w-full md:h-screen md:overflow-hidden">
            <NavSidebar className="hidden w-72 shrink-0 md:flex" />

            {mobileNavOpen && (
              <div className="fixed inset-0 z-50 md:hidden">
                <button
                  aria-label="Close navigation"
                  className="absolute inset-0 bg-black/70"
                  onClick={() => setMobileNavOpen(false)}
                />
                <NavSidebar
                  className="relative z-10 flex w-[min(86vw,20rem)] shadow-2xl"
                  onNavigate={() => setMobileNavOpen(false)}
                />
                <button
                  aria-label="Close navigation"
                  className="absolute right-4 top-4 z-20 rounded-lg border border-slate-600 bg-slate-900 p-3"
                  onClick={() => setMobileNavOpen(false)}
                >
                  <Cross1Icon />
                </button>
              </div>
            )}

            <div className="flex min-w-0 flex-1 flex-col md:h-screen">
              <header className="sticky top-0 z-40 flex h-16 shrink-0 items-center gap-3 border-b border-slate-600 bg-[#020618] px-4 md:hidden">
                <button
                  aria-label="Open navigation"
                  className="rounded-lg border border-slate-600 p-2.5"
                  onClick={() => setMobileNavOpen(true)}
                >
                  <HamburgerMenuIcon className="h-5 w-5" />
                </button>
                <img className="h-9 w-9" src="/web/makara.png" alt="PWS" />
                <span className="truncate font-semibold">PWS</span>
              </header>
              <main className="min-w-0 flex-1 overflow-y-auto">
                <Outlet />
              </main>
            </div>
          </div>
        )}
      </div>
    </AuthProvider>
  );
}
