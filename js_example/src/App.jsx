import { BrowserRouter, Routes, Route } from "react-router";
import React, { useState, useEffect } from 'react';
import Game from "./pages/Game.jsx";
import "./App.css";

import { FileUpload } from "sm64-binds-frontend";

function App() {
  const [hasRom, setHasRom] = useState(false);
  if (!hasRom) {
    return (
      <FileUpload setHasRom={setHasRom} />   
    );
  }

  return (
    <>
      <main>
        <BrowserRouter>
            <Routes>
              <Route path="/" element={<Game />}/>
            </Routes>
        </BrowserRouter>
      </main>
    </>
  );
}

export default App;