# ! [cfg_attr (rustfmt , rustfmt_skip)] # ! [allow (dead_code)] # [derive (Debug , Clone , Copy , PartialEq , Eq , PartialOrd , Ord , Hash , serde :: Serialize , serde :: Deserialize)] pub enum Numero { Uno , Due , Tre , } impl Numero { pub const ALL : & 'static [Numero] = & [Numero :: Uno , Numero :: Due , Numero :: Tre ,] ; } impl Default for Numero { fn default () -> Self { Self :: Uno } } impl std :: fmt :: Display for Numero { fn fmt (& self , f : & mut std :: fmt :: Formatter) -> std :: fmt :: Result { write ! (f , "{:?}" , self) } } impl std :: str :: FromStr for Numero { type Err = () ; fn from_str (s : & str) -> Result < Self , Self :: Err > { const STRINGS : & 'static [& 'static str] = & ["Uno" , "Due" , "Tre" ,] ; for (index , & key) in STRINGS . iter () . enumerate () { if key == s { return Ok (Numero :: ALL [index]) ; } } Err (()) } }