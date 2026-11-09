import { renderToString } from "react-dom/server"
import { App } from "./components/home/App"


export const render = () => renderToString(<App />)
