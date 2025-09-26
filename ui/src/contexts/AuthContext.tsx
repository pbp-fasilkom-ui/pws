import { useNavigate, useRouter, useSearch } from "@tanstack/react-router";
import {
  FC,
  ReactElement,
  ReactNode,
  createContext,
  useContext,
  useEffect,
  useState,
} from "react";

export const AuthContext = createContext({
  user: {
    id: "",
    username: "",
    name: "",
    attributes: {} as Record<string, any>,
  },
  authenticated: false,
  initializing: true,
  handlers: {
    login: (_username: string, _password: string) => {},
    loginWithSSO: (_ticket: string) => {},
    refreshAuthState: () => {},
  },
});

interface AuthProviderProps {
  children: ReactNode;
}

export function useAuth() {
  return useContext(AuthContext);
}

const AUTH_ROUTES = ["/login", "/register", "/sso"];

export default function AuthProvider({
  children,
}: AuthProviderProps): ReactElement<FC> {
  const [auth, setAuth] = useState({
    user: {
      id: "",
      username: "",
      name: "",
      attributes: {} as Record<string, any>,
    },
    authenticated: false,
    initializing: true,
    handlers: {
      login: (_username: string, _password: string) => {},
      loginWithSSO: (_ticket: string) => {},
      refreshAuthState: () => {},
    },
  });

  const navigate = useNavigate();
  const router = useRouter();

  const search = useSearch({
    strict: false,
  });

  const { location } = router.state;

  async function login(username: string, password: string) {
    const request = await fetch(`${import.meta.env.VITE_API_URL}/login`, {
      method: "POST",
      credentials: "include",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        username: username,
        password: password,
      }),
    });

    if (request.status >= 400) {
      const data = await request.json();
      throw data;
    }

    await refreshAuthState();
    // I know this is terrible, I hate React, please make setState awaitable holy %@!#
    // @ts-ignore
    setTimeout(
      () => navigate({ from: location.pathname, to: search?.redirect || "/" }),
      50,
    );
  }

  async function loginWithSSO(ticket: string) {
    const request = await fetch(
      `${import.meta.env.VITE_API_URL}/sso-callback`,
      {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          ticket,
          service_url: `${window.location.origin}/web/sso`,
        }),
      },
    );

    if (request.status >= 400) {
      const data = await request.json();
      throw data;
    }

    await refreshAuthState();
    // I know this is terrible, I hate React, please make setState awaitable holy %@!#
    // @ts-ignore
    setTimeout(
      () => navigate({ from: location.pathname, to: search?.redirect || "/" }),
      50,
    );
  }

  async function refreshAuthState() {
    try {
      const data = await fetch(`${import.meta.env.VITE_API_URL}/validate`, {
        credentials: "include",
      }).then((res) => {
        if (!res.ok || res.status >= 400) {
          throw Error("Failed to validate user state");
        }

        return res.json();
      });

      setAuth({
        ...auth,
        user: {
          id: data.id,
          username: data.username,
          name: data.name,
        },
        authenticated: true,
        initializing: false,
      });
    } catch (e) {
      setAuth({
        ...auth,
        user: {
          id: "",
          username: "",
          name: "",
        },
        authenticated: false,
        initializing: false,
      });
    }
  }

  useEffect(() => {
    refreshAuthState();
    const interval = setInterval(refreshAuthState, 5 * 60 * 1000);

    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    if (
      !auth.initializing &&
      !auth.authenticated &&
      !AUTH_ROUTES.some((route) => route === location.pathname)
    ) {
      router.history.replace(`/sso`);
    }
  }, [auth, location.pathname]);

  return (
    <AuthContext.Provider
      value={{
        ...auth,
        handlers: {
          login,
          loginWithSSO,
          refreshAuthState,
        },
      }}
    >
      {
        // TODO: implement proper loader
        !auth.initializing ? (
          children
        ) : (
          <div className="w-screen h-screen circle-bg"></div>
        )
      }
    </AuthContext.Provider>
  );
}

